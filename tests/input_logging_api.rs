use operation_logger::{
    InputLoggingSession, ScreenCaptureSession, start_input_logging, start_screen_capture,
};

#[test]
fn exposes_unified_input_logging_api() {
    let _start_fn = start_input_logging;
    let _stop_fn = InputLoggingSession::stop;
}

#[test]
fn exposes_screen_capture_api() {
    let _start_fn = start_screen_capture;
    let _stop_fn = ScreenCaptureSession::stop;
}
