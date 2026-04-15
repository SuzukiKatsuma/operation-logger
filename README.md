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

## 出力ファイル

### session_metadata.json

セッション単位のメタデータを保存します。

主な項目:
- `operation_logger_version`
- `is_production_build`
- `started_at_utc`
- `target_app.title`
- `target_app.process_name`

方針:
- 各ログディレクトリにつき 1 回だけ保存
- ディレクトリ名が後から変更されても、セッション開始時刻や対象アプリケーションを追跡できるようにする
- `operation_logger_version` は、このログを取得した Operation Logger のバージョン
- `is_production_build` は現在、debug / release ビルド種別に基づく真偽値

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
- 出力は 360p 固定解像度
- アスペクト比を維持し、必要に応じて黒帯を付加

### capture_metadata.csv

- `frame_index`
- `system_relative_time`
- `utc_timestamp`
- `content_width`
- `content_height`

方針:
- 動画に実際に書き込んだフレームごとに 1 行保存
- `system_relative_time` はキャプチャフレームとの時刻対応付けに使用
- `utc_timestamp` は他ログとの照合や人間向けの補助確認用
- `content_width`, `content_height` は対象ウィンドウの実コンテンツサイズを記録
- サイズ変更が生じた場合でも、動画とログを後から対応付けられるようにする

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
* log directory 生成
* session metadata 生成
* keyboard の自動リピート抑制
* controller の HID mapper
* controller の state diff
* device registry
* capture の layout / scale / timing / metadata writer

hook / Raw Input / 画面キャプチャなど OS 実環境に強く依存する部分については、unit test よりも実機確認を重視しています。

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
