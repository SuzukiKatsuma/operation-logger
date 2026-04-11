use operation_logger::{InputLoggingSession, start_input_logging};

#[test]
fn exposes_unified_input_logging_api() {
    let _start_fn = start_input_logging;
    let _stop_fn = InputLoggingSession::stop;
}
