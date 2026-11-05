# Operation Logger

Windows 上で、選択したアプリケーションへの入力ログと画面キャプチャを取得するための研究用ツールです。

現在は次の情報を記録できます。

- keyboard
- mouse button / wheel
- mouse move
- controller (Raw Input ベース)
- screen capture (対象ウィンドウの動画)

## 何をするツールか

起動中アプリケーションの一覧から対象を 1 つ選び、そのアプリケーションへの入力ログと画面キャプチャを保存します。

ログの主用途は、ゲームやアプリケーション操作の分析です。  
文字入力の意味や IME の結果を扱うのではなく、**どの入力が、いつ、どのように行われたか**、および**その時点での画面状態**を記録することを目的としています。

keyboard / mouse は low-level hook、controller は Raw Input を用いて取得します。  
画面キャプチャは、選択したトップレベルウィンドウを対象に動画として保存します。

## 出力先

ログは次のディレクトリに作成されます。

`%USERPROFILE%/Documents/OperationLogs/`

ログ開始時に、以下の形式でセッションディレクトリが作成されます。

- GUI 版：Start ボタンを押した時点で作成
- CLI 版：対象アプリケーション選択後、ログ開始と同時に作成

`YYYY-MM-DD_HHMMSS_process-name/`

例:

`2026-04-13_012345_notepad.exe`

### local_participant_id

初回起動時にローカル環境ごとの `local_participant_id` を UUID v4 として生成します。  
生成された ID は次の設定ファイルに保存され、以後の起動では同じ ID が再利用されます。

`%APPDATA%/operation-logger/config.json`

設定ファイルの形式:

```json
{
  "schema_version": 1,
  "local_participant_id": "8dd7f0c2-6e33-4ed4-a34f-0a5b7fd4b7d8",
  "created_at_utc": "2026-04-27T07:20:31.123"
}
```

`config.json` が存在しない場合は自動作成されます。  
JSON として読めない、`schema_version` が不正、`local_participant_id` が存在しない、または UUID として不正な場合は、既存ファイルを `config.invalid-YYYYMMDD_HHMMSS.json` のような名前で退避してから新しい設定を作成します。

## 出力ファイル

### session_metadata.json

セッション単位のメタデータを保存します。

主な項目:
- `operation_logger_version`
- `is_production_build`
- `local_participant_id`
- `session_id`
- `started_at_utc`
- `target_app.title`
- `target_app.process_name`

方針:
- 各ログディレクトリにつき 1 回だけ保存
- ディレクトリ名が後から変更されても、セッション開始時刻や対象アプリケーションを追跡できるようにする
- `operation_logger_version` は、このログを取得した Operation Logger のバージョン
- `is_production_build` は現在、debug / release ビルド種別に基づく真偽値
- `local_participant_id` は `%APPDATA%/operation-logger/config.json` から読み込んだローカル環境ごとの参加者 ID
- `session_id` はログ開始ごとに新しく生成される UUID v4
- `local_participant_id` と `session_id` は CSV には追加せず、`session_metadata.json` のみに保存

### keyboard_input.csv

- `timestamp`
- `virtual_key`
- `scan_code`
- `key_name`
- `event`
- `is_injected`

方針:
- 対象プロセスが foreground のときのみ記録
- IME や文字列そのものは扱わない
- `keydown` / `keyup` の状態変化のみを記録
- 自動リピートは抑制
- 左右キーを後から復元できるよう `scan_code` を保存

### mouse_input.csv

- `timestamp`
- `x`
- `y`
- `button`
- `event`
- `delta`

方針:
- カーソル直下のウィンドウが対象プロセスに属している場合のみ記録
- `x`, `y` は対象ウィンドウ基準のクライアント座標
- button: `left` / `right` / `wheel_v` / `wheel_h`
- event: `mousedown` / `mouseup` / `wheel`

### mouse_move.csv

- `timestamp`
- `x`
- `y`

方針:
- カーソル直下のウィンドウが対象プロセスに属している場合のみ記録
- 直前と同じ座標であれば記録しない

### controller_button_input.csv

- `timestamp`
- `device_id`
- `button`
- `event`

方針:
- 対象プロセスが foreground のときのみ記録
- ボタンの down / up 差分のみを記録
- button 名は機種固有名に寄らず、汎用的な名前を優先

### controller_analog_input.csv

- `timestamp`
- `device_id`
- `control`
- `value`

方針:
- 対象プロセスが foreground のときのみ記録
- analog 入力は記録前にデッドゾーン処理を実施
- stick 系 (`axis_left_x`, `axis_left_y`, `axis_right_x`, `axis_right_y`) は中心値 128 からの差分が 16 以内の場合、0 入力相当（中心値 128）として扱う
- trigger 系 (`trigger_left`, `trigger_right`) は 30 以下を 0 として扱う
- stick の閾値 16 は、Unity Input System の `defaultDeadzoneMin = 0.125` を 8-bit 生値へ近似した値（中心差分として扱うため、厳密モデルではなく近似値を採用）
- trigger の閾値 30 は、XInput の `XINPUT_GAMEPAD_TRIGGER_THRESHOLD` に準拠
- デッドゾーン処理後の値を 0〜255 の 32 分割相当で量子化してから記録
  - 量子化後の代表値は基本的に `0, 8, 16, ..., 248` で、最大値 `255` はそのまま保持
- 量子化後の値が変化したときのみ記録
- `value` は前処理 (デッドゾーン＋量子化) 後の値

### capture.mp4

対象ウィンドウの動画を保存します。

方針:
- 対象は選択したトップレベルウィンドウ
- 音声は保存しない
- 出力は 640×360、H.264/MP4、10fps 固定フレームレート（CFR）
- アスペクト比を維持し、必要に応じて黒帯を付加
- キャプチャ開始を動画の 0 秒とし、100ms ごとの時間枠に、その時刻以前の最新の実取得フレームを配置
- 新しいフレームが取得できない時間帯・停止直前の不足分は、直前の有効フレームを複製して補完
- 正常終了時の出力フレーム数は `ceil(収録経過秒数 × 10)`。再生時間は収録経過時間以上で、差は 100ms 未満
- 最初の取得前だけは、最初の実取得フレームで明示的に補完する。この区間は実際に観測した画面状態ではない（下記参照）
- 取得とエンコードは別スレッドで実行。待ち行列は最大 4 イベントとし、エンコードが追い付かない場合は新しい取得フレームを破棄するが、動画の時間枠は省略しない。破棄数はセッション情報に記録
- フレームブレンドなどの前後フレームによる補間は行わない

### capture_metadata.csv

- `frame_index`
- `system_relative_time`
- `utc_timestamp`
- `content_width`
- `content_height`

方針:
- 動画に実際に書き込んだフレームごとに 1 行保存
- `frame_index` は複製を含む動画内のフレーム番号で、0 から欠番なく連続。動画内の表示時刻（秒）は `frame_index / 10`
- `system_relative_time` は元フレームの **実取得時刻**。WGC の `SystemRelativeTime`（QPC ベース、100ns 単位）をそのまま保存
- `utc_timestamp` は同じ実取得時刻を、開始時の QPC / 高精度 UTC アンカーから変換した UTC（既存形式どおり末尾の `Z` は省略）
- この 2 つの取得時刻は **動画内の表示時刻とは異なる**。取得間隔の揺らぎ・補完があるため、表示時刻として利用しない
- 複製時は元フレームの取得時刻・UTC・コンテンツサイズをすべて保持し、`frame_index` だけを更新。同じ `system_relative_time` が連続する行から複製を識別でき、その時刻の最初の出力行まで遡ることで複製元を追跡できる
- `content_width`, `content_height` は対象ウィンドウの実コンテンツサイズを記録
- サイズ変更が生じた場合でも、動画とログを後から対応付けられるようにする

例えば、取得時刻が開始から 0.0、0.1、0.4 秒、停止が 0.6 秒なら、動画の表示時刻 0.0〜0.5 秒に対応する 6 行を保存します。各行の実取得時刻は `0.0, 0.1, 0.1, 0.1, 0.4, 0.4` 秒です。

### capture_session_metadata.json

画面収録固有の時刻と診断情報を停止時に保存します。`session_metadata.json` の `started_at_utc` は画面収録の開始時刻ではありません。

| 項目 | 意味 |
| --- | --- |
| `capture_started_at_utc`, `capture_stopped_at_utc` | キャプチャ開始・停止の UTC |
| `video_zero_at_utc` | 動画の 0 秒に対応する実時刻（キャプチャ開始と同じ） |
| `capture_started_at_system_relative_100ns`, `capture_stopped_at_system_relative_100ns` | 開始・停止の QPC 時刻（100ns 単位） |
| `elapsed_100ns`, `elapsed_seconds` | 単調増加する QPC による収録経過時間 |
| `target_fps`, `video_duration_seconds` | 固定 FPS と出力フレーム数から計算した動画再生時間 |
| `video_finalized` | MP4 の最終処理が成功したか。`false` の場合は有効な完成動画として扱わない |
| `output_frame_count` | 動画出力フレーム数。正常終了時は CSV のデータ行数・MP4 のフレーム数と一致 |
| `adopted_frame_count` | 動画で一度以上使用した実取得フレームの数（初期補完の元フレームも含む） |
| `duplicated_frame_count` | 同じ実取得フレームの 2 回目以降の出力数。`output_frame_count = adopted_frame_count + duplicated_frame_count` |
| `first_frame_system_relative_time`, `first_frame_utc` | 最初に利用した実取得フレームの取得時刻 |
| `initial_padding_policy`, `initial_padding_frame_count` | 最初の実取得フレームによる初期補完方針と、最初の取得時刻より前に表示されるフレーム数 |
| `received_frame_count`, `queue_dropped_frame_count` | WGC から取得した有効フレーム数と、待ち行列満杯で破棄した数 |
| `stale_frame_count` | 開始より前、または採用済みの取得時刻以下のため無視したフレーム数 |
| `effective_capture_fps`, `effective_adopted_fps` | 取得数 / 経過秒数、動画で使用した実取得数 / 経過秒数 |

開始は WGC の `StartCapture()` 呼び出し直前、停止は取得スレッドが停止要求（または取得エラー）を検知して取得ループを終了した時点です。Stop 操作そのものの時刻とはわずかに異なります。待ち行列の処理・停止後の補完・MP4 の最終処理時間は経過時間に含めません。UTC は同じ開始時アンカーから変換するため、収録中の OS 時計補正によって経過時間が変わることはありません。

**初期補完の注意:** 最初の取得が開始から 250ms 後なら、動画の 0、100、200ms の 3 枠は、その最初の画像で埋まります。この 3 枠は取得前の画面を観測した証拠にはなりません。CSV には 250ms 後の実取得時刻を保持し、存在しない取得時刻は生成しません。初期区間は `initial_padding_frame_count` と最初の実取得時刻で判別してください。この件数は上記の採用数・複製数と重なる件数です。初期区間以外では、新しい画像を実取得時刻より前の時間枠には使用しません。

一度も有効フレームを取得できなかった場合は、取得時刻や画面を捏造せずエラーとします。CSV はヘッダーのみ、セッション情報は出力数 0・最初の取得時刻 `null` となり、有効な MP4 は生成されません。API エラーやエンコード・書き込み失敗で異常終了したセッションの MP4 は、正常終了したものとして分析しないでください。

## 使い方

### 1. ビルド

```powershell
cargo build
```

### 2. 実行

#### GUI 版
```powershell
cargo run --bin operation-logger-gui
```

#### CLI 版
```powershell
cargo run --bin operation-logger-cli
```

### 3. 対象アプリケーションを選ぶ

#### GUI 版
起動中のアプリケーション一覧が GUI に表示されるので、一覧から対象を選択します。  
必要に応じて Refresh ボタンで一覧を更新できます。

#### CLI 版
起動中のアプリケーションが一覧表示されるので、番号を入力して対象を選択します。

### 4. ログ開始

ログ開始時にログディレクトリが作成され、入力ロギングと画面キャプチャが始まります。  
このとき、同じディレクトリに `session_metadata.json` も保存されます。

- GUI 版: Start ボタン押下で開始
- CLI 版: 対象アプリケーション選択後、そのまま開始

ログ開始ごとに新しい `session_id` が生成され、`session_metadata.json` に保存されます。  
ログディレクトリ名や各 CSV のファイル名・ヘッダーには `session_id` は含めません。

### 5. ログ停止

#### GUI 版
Stop ボタンを押すと停止します。

#### CLI 版
Enter を押すと停止します。

## 配布用実行ファイルの作成

```powershell
cargo run --bin release-package
```

## テスト

```powershell
cargo test
```

現在のテストは主に次を対象としています。

* CSV writer
* local participant config 生成・再利用・不正設定の退避
* log directory 生成
* session metadata 生成
* keyboard の自動リピート抑制
* controller の HID mapper
* controller の state diff
* device registry
* capture の layout / scale / timing / metadata writer
* WGC に依存しない CFR 時間枠・10fps / 実効 5fps・取得停止・不規則な取得間隔・停止時補完・初期補完
* Media Foundation で実際に MP4 を生成・デコードし、フレーム数 / PTS / SampleDuration と CSV / セッション情報の対応を検証

hook / Raw Input / 画面キャプチャなど OS 実環境に強く依存する部分については、unit test よりも実機確認を重視しています。

### 画面録画の実機確認

1. Windows の Visual Studio Developer PowerShell（C++ Build Tools と Windows SDK 導入済み）で `cargo fmt --check`、`cargo test` を実行する。
2. `cargo run --bin operation-logger-gui` で起動し、画面上に経過時間を表示するアプリを対象に 10 秒以上録画する。途中で静止状態を 1 秒以上作り、可能なら最小化・復帰やサイズ変更も試す。
3. Stop 後の `capture_session_metadata.json` で `elapsed_seconds` と `video_duration_seconds` の差が 0 以上 0.1 秒未満、`output_frame_count` が `ceil(elapsed_seconds * 10)` と一致することを確認する。
4. `capture_metadata.csv` のデータ行数と `output_frame_count` の一致、0 からの連続番号、静止・取得停止区間で取得時刻とコンテンツサイズが保持されることを確認する。通常の静止状態でも OS がフレームを供給する場合があるため、複製の発生は CSV の取得時刻で判断する。
5. ffprobe が利用可能なら次を実行し、`nb_read_frames` と CSV 行数、`avg_frame_rate` と `10/1`、解像度と `640×360`、`duration` とセッション情報を照合する。

   ```powershell
   ffprobe -v error -select_streams v:0 -count_frames -show_entries stream=codec_name,width,height,avg_frame_rate,nb_read_frames,duration -of json "<ログディレクトリ>/capture.mp4"
   ```

6. 動画を再生し、取得停止区間で直前の画像が保持され、復帰後の画像が取得時刻より前に表示されないことを確認する。表示時刻は `video_zero_at_utc + frame_index / 10` から求め、CSV の UTC は元画像の取得時刻として照合する。最初の取得前の区間は上記の初期補完として扱う。

## 既知の制約

* Windows 専用
* controller 実装は汎用 HID parser ではない
* controller の `device_id` は Raw Input の device handle ベースであり、同一実行セッション内の識別子
* OS 再起動や再接続をまたぐ永続 ID ではない
* controller の profile は現状 DualSense を主対象とする
* 画面キャプチャは対象ウィンドウ単位
* 画面キャプチャの詳細実装は Windows 固有 API に依存
* サイズ変更には対応するが、出力動画は固定解像度で保存
* `session_metadata.json` の `is_production_build` は現在、debug / release ビルド種別をもとに決定

## 今後の予定

* GUI 版の改善
* controller profile の分離

## 位置づけ

本プロジェクトは、研究・実験用途を主目的とした入力ログ取得ツールです。  
汎用配布ライブラリとしての完成度よりも、入力取得と分析のための記録基盤としての実用性を優先しています。
