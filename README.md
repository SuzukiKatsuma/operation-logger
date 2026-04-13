# Operation Logger

Windows 上で、選択したアプリケーションに対する入力ログと画面キャプチャを取得するための研究用ツールです。

現在は次を記録できます。

- keyboard
- mouse button / wheel
- mouse move
- controller (Raw Input ベース)
- screen capture (対象ウィンドウの動画)

## 何をするツールか

起動中アプリケーションの一覧から対象アプリケーションを 1 つ選び、そのアプリケーションに対する入力ログおよび画面キャプチャを保存します。

ログの主用途は、ゲームやアプリケーション操作の分析です。  
文字入力の意味や IME の結果を扱うのではなく、**どの入力が、いつ、どのように行われたか**、および **その時点でどのような画面状態だったか** を記録することを目的としています。

## 現在の前提

- Windows 専用です
- keyboard / mouse は low-level hook を使っています
- controller は Raw Input を使っています
- controller 実装は現状 **DualSense での利用を主対象にした最小実装**です
- controller の一次ログでは、ボタンの意味づけやデッドゾーン処理は行いません
- controller の analog 値は **raw_value** を保存し、正規化や丸め込みは後段分析で行う前提です
- 画面キャプチャは **選択したトップレベルウィンドウ** を対象に行います
- 画面キャプチャは **動画保存** を前提とし、静止画保存は行いません
- 画面キャプチャの時刻同期は、映像への焼き込みではなく metadata CSV により行います
- 各ログディレクトリには、セッション単位の補助情報として `session_metadata.json` を保存します

## 出力先

ログは次の配下に作成されます。

`%USERPROFILE%/Documents/OperationLogs/`

実際には、対象アプリケーションを選択した時点で、次のようなディレクトリが作成されます。

`YYYY-MM-DD_HHMMSS_process-name/`

例:

`2026-04-13_012345_notepad.exe`

## 出力されるファイル

### session_metadata.json

セッション単位のメタデータを保存します。

主な項目:
- `operation_logger_version`
- `is_production_build`
- `started_at_utc`
- `target_app.title`
- `target_app.process_name`

方針:
- 各ログディレクトリにつき 1 回だけ保存します
- ディレクトリ名が後で変更されても、セッション開始時刻や対象アプリケーションを追跡できるようにします
- `operation_logger_version` は、このログを取得した Operation Logger のバージョンです
- `is_production_build` は、現在は debug / release ビルド種別に基づく真偽値です

### keyboard_input.csv

- `timestamp`
- `virtual_key`
- `scan_code`
- `key_name`
- `event`
- `is_injected`

方針:
- IME や文字列そのものは扱いません
- `keydown` / `keyup` の状態変化のみを記録します
- 自動リピートは抑制します
- 左右キーの復元が後からできるように `scan_code` を保存します

### mouse_input.csv

- `timestamp`
- `x`
- `y`
- `button`
- `event`
- `delta`

方針:
- 座標は対象アプリケーションのクライアント座標です
- button は `left` / `right` / `wheel_v` / `wheel_h`
- event は `mousedown` / `mouseup` / `wheel`

### mouse_move.csv

- `timestamp`
- `x`
- `y`

方針:
- 直前と同じ座標であれば記録しません

### controller_button_input.csv

- `timestamp`
- `device_id`
- `button`
- `event`

方針:
- selected process が foreground のときだけ記録します
- ボタンの down / up 差分のみを記録します
- button 名は機種固有名に寄せすぎず、汎用的な名前を優先しています

### controller_analog_input.csv

- `timestamp`
- `device_id`
- `control`
- `raw_value`

方針:
- selected process が foreground のときだけ記録します
- 値が変化したときだけ記録します
- 一次ログには `raw_value` のみを保存します
- デッドゾーン処理や正規化は後段の分析で行う前提です

### capture.mp4

対象ウィンドウの動画を保存します。

方針:
- 対象は選択したトップレベルウィンドウです
- 音声は保存しません
- 出力は 360p 固定解像度です
- アスペクト比は維持し、必要に応じて黒帯を入れます

### capture_metadata.csv

- `frame_index`
- `system_relative_time`
- `utc_timestamp`
- `content_width`
- `content_height`

方針:
- metadata は、**動画に実際に書き込んだフレームごと**に 1 行保存します
- `system_relative_time` はキャプチャフレーム時刻との対応付けに使うことを目的としています
- `utc_timestamp` は他のログとの照合や人間向けの補助的な確認用です
- `content_width`, `content_height` は対象ウィンドウの実コンテンツサイズを保存します
- サイズ変更が発生した場合でも、後から動画とログを対応付けられるようにします

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
起動中のアプリケーション一覧が GUI 上に表示されるので、一覧から対象アプリケーションを選択します。
必要に応じて Refresh ボタンで一覧を更新できます。

#### CLI 版
起動中のアプリケーションが一覧表示されるので、番号を入力して対象アプリケーションを選択します。

### 4. ログ開始

対象選択後にログディレクトリが作成され、入力ロギングと画面キャプチャが始まります。
このとき、同じディレクトリに `session_metadata.json` も保存されます。

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

現在のテストは主に次を対象にしています。

* CSV writer
* log directory 生成
* session metadata 生成
* keyboard の自動リピート抑制
* controller の HID mapper
* controller の state diff
* device registry
* capture の layout / scale / timing / metadata writer

OS 実環境に強く依存する部分、特に hook / Raw Input / 画面キャプチャについては、unit test よりも実機確認を重視しています。

## 既知の制約

* Windows 専用です
* controller 実装は汎用 HID parser ではありません
* controller の `device_id` は Raw Input の device handle ベースであり、同一実行 session 内の識別子です
* OS 再起動や再接続をまたぐ永続 ID ではありません
* controller の profile は現状 DualSense を主対象にしています
* foreground 条件を満たさないと controller 入力は記録しません
* 画面キャプチャは対象ウィンドウベースです
* 画面キャプチャの詳細実装は Windows 固有 API に依存します
* サイズ変更には対応しますが、出力動画は固定解像度で保存されます
* 映像への時刻焼き込みは行わず、`capture_metadata.csv` により同期します
* `session_metadata.json` の `is_production_build` は現在、debug / release ビルド種別をもとに決定しています

## 今後の予定

* GUI 版の改善
* controller profile の分離

## 位置づけ

このプロジェクトは、研究用途・実験用途を主目的とした入力ログ取得ツールです。
汎用配布ライブラリとしての完成度よりも、入力取得と分析のための記録基盤としての実用性を優先しています。
