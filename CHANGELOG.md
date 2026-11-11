## 0.3.0 (2026-11-11)

* Switched to monitor-based capture with cropping to the selected application window.
* Added support for recording overlapping menus, dialogs, and popups within the target window area.
* Added dynamic cropping to follow window movement and resizing.
* Added `window_x` and `window_y` to `capture_metadata.csv` while preserving existing input log formats.
* Improved capture efficiency with GPU-based cropping and reusable staging textures.
* Added tests for window geometry, DPI handling, and monitor-based capture.

## 0.2.1 (2026-11-05)

* Fixed MP4 recordings being shorter than the actual capture duration when the effective capture frame rate dropped below 10 FPS.
* Maintained a constant 10 FPS video timeline by duplicating frames during capture gaps.
* Preserved original capture timestamps for duplicated frames without changing the `capture_metadata.csv` schema.
* Added `capture_session_metadata.json` to record capture start/end times and frame statistics.
* Separated frame capture and video encoding into independent threads.
* Added tests for video duration and metadata consistency, including Media Foundation decoding tests.

## 0.2.0 (2026-04-30)

* Added persistent local participant configuration at `%APPDATA%\operation-logger\config.json`.
* Added `local_participant_id` and per-session `session_id` to `session_metadata.json` without changing CSV schemas.
* Displayed the full `local_participant_id` in both GUI and CLI.
* Backed up invalid local participant config files before regenerating them.

## 0.1.0 (2026-04-16)

* Initial public release.