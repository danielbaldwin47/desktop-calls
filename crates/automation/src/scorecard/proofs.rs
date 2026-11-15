//! Which of the harness's scenarios prove each item against the Simulated
//! Phone, by their full names in the scenarios' test binary
//! (`<file>::<function>`). A name that is renamed or removed fails its item
//! rather than drop out of the count.

/// Item 1: scenarios that hold the Banner to 1 s from the Call appearing,
/// by the app's log (`assert_banner_within`).
pub const BANNER: &[&str] = &[
    "incoming_call_surfaces::the_banner_shows_the_formatted_number_within_a_second",
    "incoming_call_surfaces::clicking_answer_answers_and_the_call_window_shows_the_audio_here_and_a_timer",
    "incoming_call_surfaces::a_wideband_call_shows_hd",
    "contacts::with_5000_contacts_the_banner_still_shows_within_a_second",
    "keybinding::while_ringing_the_keybinding_focuses_the_banner_and_a_answers_on_the_desktop",
    "keybinding::while_ringing_the_keybinding_focuses_the_banner_and_d_declines",
    "keybinding::enter_on_the_banner_with_the_keyboard_answers_and_never_declines",
    "keybinding::escape_gives_focus_back_and_the_call_keeps_ringing",
    "do_not_disturb::scenario_16_under_do_not_disturb_the_banner_shows_and_the_desktop_stays_silent",
    "do_not_disturb::turning_do_not_disturb_on_mid_ring_silences_the_desktop_and_off_rings_it_again",
    "answered_on_phone_surfaces::answered_on_the_phone_the_banner_closes_and_the_call_window_says_audio_on_phone",
    "answered_on_phone_surfaces::declined_on_the_phone_the_banner_closes_at_once",
];

/// Item 2: scenarios that hold the audio to 1.5 s from Answer, or from a
/// Handoff's request, by the app's log.
pub const AUDIO: &[&str] = &[
    "incoming_call::answering_answers_then_clears_reject_sco_then_activates_in_order",
    "incoming_call::consecutive_calls_on_the_same_path_are_different_calls",
    "incoming_call::a_wideband_call_reports_wideband",
    "keybinding::while_ringing_the_keybinding_focuses_the_banner_and_a_answers_on_the_desktop",
    "incoming_call_surfaces::clicking_answer_answers_and_the_call_window_shows_the_audio_here_and_a_timer",
    "crash_restart::after_a_crash_mid_call_the_call_window_shows_the_same_call_audio_and_timer",
    "telephony_restart::a_restart_mid_call_keeps_the_call_in_the_call_window_and_hang_up_still_works",
    "handoff::the_audio_goes_here_to_the_phone_here_and_to_the_phone_again_in_one_call",
    "handoff::an_incoming_call_answered_on_the_phone_can_move_its_audio_here",
    "handoff::clicking_the_handoff_button_moves_the_audio_here_and_back_to_the_phone",
    "handoff::move_audio_desktop_brings_the_audio_here_and_sets_reject_sco_again",
];

/// Item 6: scenarios that hold reconnecting to 10 s, out of range and
/// across suspend and resume.
pub const RECONNECT: &[&str] = &[
    "out_of_range::back_in_range_the_phone_is_connected_within_10_s_and_contacts_and_call_history_refresh",
    "out_of_range::after_resume_the_phone_is_connected_within_10_s",
    "out_of_range::after_resume_the_phone_is_connected_within_10_s_when_the_app_had_already_paged_it",
    "out_of_range::when_the_desktops_bluetooth_comes_back_on_the_phone_is_connected_within_10_s",
    "out_of_range::hands_free_dropped_with_the_phone_still_connected_comes_back_within_10_s",
    "out_of_range::after_a_connect_that_is_never_answered_the_phone_back_in_range_is_connected_within_10_s",
    "out_of_range::after_a_connect_that_is_never_answered_hands_free_dropped_comes_back_within_10_s",
];

/// Item 7: the keyboard-only suite (#25).
pub const KEYBOARD: &[&str] = &[
    "keyboard_only::every_action_is_performed_by_a_case_or_names_its_ticket",
    "keyboard_only::the_banner_by_keys_alone",
    "keyboard_only::the_call_window_and_the_call_pill_by_keys_alone",
    "keyboard_only::call_waiting_by_keys_alone",
    "keyboard_only_dialer::the_dialer_and_the_quiet_hours_window_by_keys_alone",
    "keyboard_only_dialer::while_calls_are_off_the_dialer_calls_anyway_by_keys_alone",
    "keyboard_only_setup::first_run_in_the_setup_window_by_keys_alone",
    "keyboard_only_setup::a_refused_phone_then_skipped_contacts_by_keys_alone",
    "keyboard_only_setup::a_setup_window_fix_that_copies_a_command_by_keys_alone",
];

/// The keyboard-only suite's list of actions, to count those with no key
/// path yet: the suite passes with them, listed against their tickets, so
/// the scorecard reads them here.
pub const KEYBOARD_SUITE: &str = include_str!("../../../test-harness/tests/keyboard_only.rs");

/// Item 5: the spec's 20 canonical scenarios, each with the scenarios that
/// prove it; it passes when every one of them passed.
pub const CANONICAL: [(&str, &[&str]); 20] = [
    (
        "First-run pair and sync",
        &[
            "setup_contacts::scenario_1_pair_sync_open_the_dialer_with_the_keybinding_and_place_a_call",
        ],
    ),
    (
        "Contacts prompt times out at 30 s, then retry succeeds",
        &[
            "setup_contacts::an_ignored_prompt_runs_the_countdown_to_zero_and_try_again_asks_again",
            "contacts::an_unanswered_prompt_needs_permission_until_the_phone_gives_up",
        ],
    ),
    (
        "Contacts denied, with Calls still working",
        &[
            "setup_contacts::denied_the_window_says_calls_still_work_and_goes_on_to_the_finish_page",
            "contacts::denied_contacts_leave_a_problem_and_calls_still_work_by_number",
        ],
    ),
    (
        "Answer on the Desktop",
        &["incoming_call::answering_answers_then_clears_reject_sco_then_activates_in_order"],
    ),
    (
        "Answer on the Phone",
        &[
            "incoming_call::answered_on_the_phone_the_audio_stays_on_the_phone",
            "answered_on_phone_surfaces::answered_on_the_phone_the_banner_closes_and_the_call_window_says_audio_on_phone",
        ],
    ),
    (
        "Decline on the Desktop",
        &["incoming_call::declining_hangs_up_the_ringing_call"],
    ),
    (
        "Declined on the Phone",
        &["incoming_call::declined_on_the_phone_the_desktop_stops_ringing_at_once"],
    ),
    (
        "Caller hangs up: a missed notification, whose click calls back",
        &[
            "missed_calls::scenario_8_a_caller_who_hangs_up_leaves_a_missed_call_notification_whose_click_calls_back",
        ],
    ),
    (
        "Withheld number, ringing then missed",
        &[
            "missed_calls::scenario_9_a_withheld_number_rings_then_is_missed_and_cant_be_called_back",
        ],
    ),
    (
        "Dial from a Dialer search",
        &[
            "dialer_surface::typing_a_dialable_number_shows_a_call_row_and_enter_dials_it",
            "dialer_surface::down_down_enter_dials_the_selected_row_and_the_dialer_closes_as_the_call_starts",
        ],
    ),
    (
        "Call placed on the Phone",
        &["handoff::a_call_placed_on_the_phone_shows_calling_and_leaves_the_audio_there"],
    ),
    (
        "Handoff both ways",
        &["handoff::the_audio_goes_here_to_the_phone_here_and_to_the_phone_again_in_one_call"],
    ),
    (
        "Mute and keypad tones",
        &[
            "mute_and_tones::set_muted_sets_the_microphone_volume_to_zero_and_back",
            "mute_and_tones::send_tones_plays_each_tone_on_the_phone_in_order",
            "call_window_keyboard::every_action_in_the_call_window_is_reachable_by_the_shortcut_it_shows",
        ],
    ),
    (
        "Call waiting: Hold & answer, Swap, hang up one",
        &[
            "call_waiting::scenario_14_hold_and_answer_then_swap_then_hanging_up_one_leaves_the_other_live",
        ],
    ),
    (
        "Call waiting: End & answer, and Decline waiting",
        &[
            "call_waiting::scenario_15_end_and_answer_ends_the_live_call_and_connects_the_waiting_one",
            "call_waiting::scenario_15_declining_the_waiting_call_hangs_up_only_that_call",
        ],
    ),
    (
        "Do Not Disturb: Banner shown, no ring",
        &[
            "do_not_disturb::scenario_16_under_do_not_disturb_the_banner_shows_and_the_desktop_stays_silent",
        ],
    ),
    (
        "Out of range and back, with re-sync",
        &[
            "out_of_range::back_in_range_the_phone_is_connected_within_10_s_and_contacts_and_call_history_refresh",
        ],
    ),
    (
        "Suspend and resume",
        &[
            "out_of_range::after_resume_the_phone_is_connected_within_10_s",
            "out_of_range::after_resume_the_phone_is_connected_within_10_s_when_the_app_had_already_paged_it",
        ],
    ),
    (
        "Telephony service restart, while idle and mid-Call",
        &[
            "telephony_restart::a_restart_while_idle_comes_back_connected_and_keeps_audio_on_the_phone",
            "telephony_restart::a_restart_mid_call_keeps_the_call_in_the_call_window_and_hang_up_still_works",
        ],
    ),
    (
        "Earbuds: audio routed to other hands-free equipment",
        &[
            "earbuds::with_earbuds_connected_the_banner_appears_and_answering_here_brings_the_audio_here",
            "earbuds::a_call_answered_on_the_earbuds_shows_its_audio_on_the_phone",
            "earbuds::a_phone_that_keeps_the_audio_on_the_earbuds_gets_a_plain_message_and_the_call_stays_live",
        ],
    ),
];

/// Item 5 too: a scenario for each quirk and setup problem the spec names.
pub const QUIRKS_AND_PROBLEMS: &[(&str, &[&str])] = &[
    (
        "Call1.Hangup missing, with the oFono fallback",
        &["incoming_call::hanging_up_falls_back_to_ofono_when_call1_hangup_does_not_exist"],
    ),
    (
        "Too few arguments on a redundant hangup",
        &["incoming_call::too_few_arguments_while_a_call_tears_down_means_it_ended"],
    ),
    (
        "GetCalls empty while dialling",
        &[
            "outgoing_call_surfaces::while_get_calls_is_empty_during_dialling_the_call_window_never_says_call_ended",
        ],
    ),
    (
        "Call path reuse",
        &["incoming_call::consecutive_calls_on_the_same_path_are_different_calls"],
    ),
    (
        "Answered incoming reports active",
        &["incoming_call::answering_answers_then_clears_reject_sco_then_activates_in_order"],
    ),
    (
        "Withheld as empty, as \"withheld\" and as \"\",129",
        &[
            "missed_calls::scenario_9_a_withheld_number_rings_then_is_missed_and_cant_be_called_back",
        ],
    ),
    (
        "One obexd session at a time",
        &["contacts::pbap_sessions_never_overlap"],
    ),
    (
        "Call History with no timezone",
        &["contacts::call_history_times_without_a_zone_are_desktop_local_time"],
    ),
    (
        "Call History duplicates across books",
        &["contacts::without_cch_the_call_history_merges_incoming_outgoing_and_missed"],
    ),
    (
        "Call History with one unsupported book",
        &["contacts::a_folder_the_phone_lacks_does_not_stop_the_others"],
    ),
    (
        "vCard 2.1 quoted-printable and folded lines",
        &["contacts::quoted_printable_and_folded_vcard_2_1_names_decode"],
    ),
    (
        "No Deactivate()",
        &["handoff::the_transport_has_no_deactivate_and_the_handoff_to_the_phone_needs_none"],
    ),
    (
        "Contacts prompt timeout",
        &["contacts::an_unanswered_prompt_needs_permission_until_the_phone_gives_up"],
    ),
    (
        "Contacts prompt Forbidden",
        &["contacts::denied_contacts_leave_a_problem_and_calls_still_work_by_number"],
    ),
    (
        "Introspect with no children",
        &["conformance::introspect_lists_no_children_and_get_modems_has_empty_property_lists"],
    ),
    (
        "ag1 indexing",
        &["connection::the_phone_link_finds_audio_gateways_only_through_the_object_manager"],
    ),
    (
        "AudioGateway only after the hands-free connection",
        &["connection::hands_free_drops_and_returns"],
    ),
    (
        "RejectSCO cleared before Activate()",
        &["incoming_call::answering_answers_then_clears_reject_sco_then_activates_in_order"],
    ),
    (
        "Codec 0 while idle",
        &["incoming_call::an_incoming_call_rings_on_the_desktop"],
    ),
    (
        "Phone refuses hold or conference in invalid states",
        &[
            "call_waiting::when_the_phone_refuses_to_hold_the_user_is_told_and_both_calls_stay_as_they_were",
            "call_waiting::swap_with_no_call_on_hold_is_refused_with_the_reason",
        ],
    ),
    (
        "BlueZ experimental Telephony1 conflict",
        &["setup_checks::bluez_hands_free_on_shows_the_conflict_and_how_to_turn_it_off"],
    ),
    (
        "Setup problem: Bluetooth off",
        &["setup_checks::bluetooth_off_shows_turn_on_and_pressing_it_turns_bluetooth_on"],
    ),
    (
        "Setup problem: bluez-obex missing",
        &[
            "setup_checks::contacts_support_missing_shows_the_install_command_and_copy_puts_it_on_the_clipboard",
        ],
    ),
    (
        "Setup problem: Phone calls off",
        &["setup_window::a_phone_refusing_hands_free_shows_where_to_switch_phone_calls_on"],
    ),
    (
        "Setup problem: pairing lost",
        &["out_of_range::a_phone_that_forgets_the_pairing_is_a_problem_not_out_of_range"],
    ),
];

/// Every scenario named above, once each, in the order first named.
pub fn every_scenario() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    let lists = [BANNER, AUDIO, RECONNECT, KEYBOARD]
        .into_iter()
        .chain(CANONICAL.iter().map(|(_, names)| *names))
        .chain(QUIRKS_AND_PROBLEMS.iter().map(|(_, names)| *names));
    for name in lists.flatten() {
        if !names.contains(name) {
            names.push(name);
        }
    }
    names
}
