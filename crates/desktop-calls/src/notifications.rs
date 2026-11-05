//! Missed Call notifications, through the desktop's notification server
//! (`org.freedesktop.Notifications`).
//!
//! Each caller has one notification: a later Missed Call from the same
//! caller replaces it in place (`replaces_id`). Clicking it (its `default`
//! action), its Call back and Dismiss buttons, and the user closing it come
//! back as inputs. If no notification server answers, the failure is logged
//! and the app goes on.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use desktop_calls_core::messages::{self, Label};
use desktop_calls_core::{CallId, Caller, Input, MissedCallNotice};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;
use glib::MainContext;
use zbus::message::Type;

use zbus::{Connection, MatchRule, MessageStream, Variant};

use crate::control::Publisher;
use crate::logging::EventKind;
use crate::surfaces;

const NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const APP_NAME: &str = "Desktop Calls";
const ICON: &str = "call-missed-symbolic";
/// The desktop file, so the notification server can find the app's name,
/// icon and settings.
const DESKTOP_ENTRY: &str = "io.github.danielbaldwin47.DesktopCalls";
/// The notification specification's category for a Missed Call.
const CATEGORY: &str = "call.unanswered";
/// Normal urgency: a Missed Call follows Do Not Disturb.
const URGENCY_NORMAL: u8 = 1;
/// Let the notification server choose how long the notification shows.
const SERVER_TIMEOUT: i32 = -1;

/// The action keys.
const DEFAULT: &str = "default";
const CALL_BACK: &str = "call-back";
const DISMISS: &str = "dismiss";

/// `NotificationClosed`'s reason when the user dismissed the notification.
const CLOSED_BY_USER: u32 = 2;

/// The notifications shown: by the first Missed Call each tells of, the
/// notification server's id for it.
type Shown = Rc<RefCell<HashMap<CallId, u32>>>;

/// Shows Missed Call notifications, and reports what the user does with
/// them.
pub struct Notifier {
    notices: UnboundedSender<MissedCallNotice>,
}

impl Notifier {
    /// Starts following the notification server on `session`, reporting
    /// the user's actions to `inputs`. Each Missed Call is signalled on
    /// the control interface through `publisher` once its notification is
    /// shown, or has failed to show.
    pub fn start(
        session: &Connection,
        inputs: UnboundedSender<Input>,
        publisher: Publisher,
    ) -> Self {
        let shown = Shown::default();
        let (notices, queued) = unbounded();
        let context = MainContext::default();
        context.spawn_local(follow(session.clone(), shown.clone(), inputs));
        context.spawn_local(show_all(session.clone(), queued, shown, publisher));
        Self { notices }
    }

    /// Shows `notice`, replacing its caller's notification if still shown.
    /// Never waits on the notification server.
    pub fn notify(&self, notice: MissedCallNotice) {
        // The queue only closes with the main context.
        let _ = self.notices.unbounded_send(notice);
    }
}

/// Shows each notice in turn, so a repeat always knows the id of the
/// notification it replaces.
async fn show_all(
    session: Connection,
    mut queued: UnboundedReceiver<MissedCallNotice>,
    shown: Shown,
    publisher: Publisher,
) {
    while let Some(notice) = queued.next().await {
        let replaces = shown
            .borrow()
            .get(&notice.notification)
            .copied()
            .unwrap_or(0);
        match show(&session, &notice, replaces).await {
            Ok(id) => {
                shown.borrow_mut().insert(notice.notification, id);
                tracing::info!(
                    event_kind = %EventKind::MissedCall,
                    call_id = notice.call.0,
                    count = notice.count,
                    "Notified a Missed Call"
                );
            }
            Err(error) => tracing::warn!(
                event_kind = %EventKind::NotificationFailed,
                error_kind = "notify-failed",
                call_id = notice.call.0,
                detail = %error,
                "Couldn't show the Missed Call notification"
            ),
        }
        if let Err(error) = publisher.missed_call(&notice).await {
            tracing::warn!(
                event_kind = %EventKind::PublishFailed,
                error_kind = "publish-failed",
                detail = %error,
                "Couldn't signal the Missed Call on the control interface"
            );
        }
    }
}

async fn show(session: &Connection, notice: &MissedCallNotice, replaces: u32) -> zbus::Result<u32> {
    let text = messages::missed_call(notice, surfaces::now(), surfaces::utc_offset_seconds());
    let mut actions = vec![DEFAULT, messages::label(Label::ShowInDialer)];
    if matches!(notice.caller, Caller::Number(_)) {
        actions.extend([CALL_BACK, messages::label(Label::CallBack)]);
    }
    actions.extend([DISMISS, messages::label(Label::Dismiss)]);
    let hints = HashMap::from([
        ("category", Variant::from(CATEGORY)),
        ("desktop-entry", Variant::from(DESKTOP_ENTRY)),
        ("urgency", Variant::from(URGENCY_NORMAL)),
    ]);
    let reply = session
        .call_method(
            Some(NAME),
            PATH,
            Some(NAME),
            "Notify",
            &(
                APP_NAME,
                replaces,
                ICON,
                text.summary.as_str(),
                text.body.as_str(),
                actions,
                hints,
                SERVER_TIMEOUT,
            ),
        )
        .await?;
    reply.body().deserialize()
}

/// Turns the notification server's `ActionInvoked` and `NotificationClosed`
/// for the notifications shown into inputs.
async fn follow(session: Connection, shown: Shown, inputs: UnboundedSender<Input>) {
    let stream = match subscribe(&session).await {
        Ok(stream) => stream,
        Err(error) => {
            tracing::warn!(
                event_kind = %EventKind::NotificationFailed,
                error_kind = "subscribe-failed",
                detail = %error,
                "Couldn't follow the notification server, so clicking a Missed Call notification won't work"
            );
            return;
        }
    };
    let mut stream = stream;
    while let Some(Ok(message)) = stream.next().await {
        let header = message.header();
        let input =
            match header.member().map(zbus::names::MemberName::as_str) {
                Some("ActionInvoked") => message
                    .body()
                    .deserialize::<(u32, String)>()
                    .ok()
                    .and_then(|(id, key)| {
                        let notification = forget(&shown, id)?;
                        match key.as_str() {
                            DEFAULT => Some(Input::OpenMissedCall(notification)),
                            CALL_BACK => Some(Input::CallBack(notification)),
                            DISMISS => Some(Input::MissedCallDismissed(notification)),
                            _ => None,
                        }
                    }),
                // Expired, or closed some other way: the next Missed Call from
                // the caller still counts this one.
                Some("NotificationClosed") => message
                    .body()
                    .deserialize::<(u32, u32)>()
                    .ok()
                    .filter(|(_, reason)| *reason == CLOSED_BY_USER)
                    .and_then(|(id, _)| forget(&shown, id))
                    .map(Input::MissedCallDismissed),
                _ => None,
            };
        if let Some(input) = input {
            tracing::info!(
                event_kind = %EventKind::MissedCallAction,
                "The user acted on a Missed Call notification: {input:?}"
            );
            if inputs.unbounded_send(input).is_err() {
                return;
            }
        }
    }
}

async fn subscribe(session: &Connection) -> zbus::Result<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path(PATH)
        .interface(NAME)
        .build()?;
    MessageStream::for_match_rule(rule, session, None).await
}

/// The notification the server knows as `id`, no longer shown.
fn forget(shown: &Shown, id: u32) -> Option<CallId> {
    let mut shown = shown.borrow_mut();
    let notification = shown
        .iter()
        .find(|(_, shown_id)| **shown_id == id)
        .map(|(notification, _)| *notification)?;
    shown.remove(&notification);
    Some(notification)
}
