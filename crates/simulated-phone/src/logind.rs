//! logind's `PrepareForSleep` signal, which the Desktop follows to connect
//! the Phone again at once after suspend (ticket #19). The Simulated Phone
//! stands in for logind on the private system bus, so tests can suspend
//! and resume the Desktop without touching the real one.

use zbus::Connection;

const SERVICE: &str = "org.freedesktop.login1";
const PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";

/// Takes logind's name on a system bus connection of its own, as logind is
/// a process of its own.
pub async fn connect() -> zbus::Result<Connection> {
    let connection = zbus::connection::Builder::system().build().await?;
    connection.request_name(SERVICE).await?;
    Ok(connection)
}

/// Sends `PrepareForSleep(start)`: `true` as the Desktop suspends, `false`
/// once it has resumed.
pub async fn prepare_for_sleep(logind: &Connection, start: bool) -> zbus::Result<()> {
    logind
        .emit_signal(None::<&str>, PATH, MANAGER, "PrepareForSleep", &(start,))
        .await
}
