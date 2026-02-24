# cargo-sysroot

[![shiguredo_sysroot](https://img.shields.io/crates/v/shiguredo_sysroot.svg)](https://crates.io/crates/shiguredo_sysroot)
[![Documentation](https://docs.rs/shiguredo_sysroot/badge.svg)](https://docs.rs/shiguredo_sysroot)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## About Shiguredo's open source software

We will not respond to PRs or issues that have not been discussed on Discord. Also, Discord is only available in Japanese.

Please read <https://github.com/shiguredo/oss> before use.

## 時雨堂のオープンソースソフトウェアについて

利用前に <https://github.com/shiguredo/oss> をお読みください。

## 概要

Rust のクロスコンパイル用 sysroot を生成し、`.cargo/config.toml` を自動設定する Cargo サブコマンドです。

JSON 設定ファイルに基づいて Debian/Ubuntu の APT リポジトリからパッケージをダウンロードし、sysroot を構築します。

## 特徴

- JSON 設定ファイルによる宣言的な sysroot 定義
- APT リポジトリからのパッケージダウンロードと展開
- `.cargo/config.toml` の自動更新 (linker, rustflags, CC/CXX 環境変数)
- CC/CXX ラッパースクリプトの自動生成
- sysroot パスの相対パス化

## 必要なもの

- Rust 1.88 以上
- `dpkg-deb` コマンド (deb パッケージの展開に使用)

## インストール

```bash
cargo install shiguredo_sysroot
```

## 使い方

### 設定ファイルの作成

JSON 形式の設定ファイルを作成します。

```json
{
  "name": "ubuntu-24.04_armv8",
  "arch": "arm64",
  "rust_target": "aarch64-unknown-linux-gnu",
  "linker": "aarch64-linux-gnu-gcc",
  "packages": [
    "libc6-dev",
    "libstdc++-13-dev"
  ],
  "repos": [
    {
      "url": "http://ports.ubuntu.com/ubuntu-ports",
      "suites": ["noble"],
      "components": ["main", "universe"]
    }
  ]
}
```

| フィールド | 説明 |
|---|---|
| `name` | sysroot の識別名 (`[a-zA-Z0-9._-]` のみ) |
| `arch` | APT アーキテクチャ名 (例: `arm64`, `armhf`, `riscv64`) |
| `rust_target` | Rust のターゲットトリプル |
| `linker` | クロスコンパイラのリンカ |
| `packages` | インストールするパッケージ一覧 |
| `repos` | APT リポジトリの定義 (url, suites, components) |

Raspberry Pi OS (trixie) 向けの例です。Debian ベースのリポジトリと Raspberry Pi 独自リポジトリの両方を指定します。

```json
{
  "name": "raspberry-pi-os_armv8",
  "arch": "arm64",
  "rust_target": "aarch64-unknown-linux-gnu",
  "linker": "aarch64-linux-gnu-gcc",
  "packages": [
    "libc6-dev",
    "libstdc++-14-dev"
  ],
  "repos": [
    {
      "url": "http://deb.debian.org/debian",
      "suites": ["trixie"],
      "components": ["main"]
    },
    {
      "url": "http://archive.raspberrypi.com/debian",
      "suites": ["trixie"],
      "components": ["main"]
    }
  ]
}
```

### 実行

```bash
cargo shiguredo-sysroot --config ubuntu-24.04_armv8.json
```

実行すると以下が行われます:

1. 設定ファイルに従い APT リポジトリからパッケージをダウンロード
2. `target/shiguredo-sysroot/<name>/sysroot/` に sysroot を構築
3. `target/shiguredo-sysroot/<name>/bin/` に CC/CXX ラッパースクリプトを生成
4. `.cargo/config.toml` に linker, rustflags, CC/CXX 環境変数を設定

設定完了後、以下のようにクロスコンパイルできます:

```bash
cargo build --target aarch64-unknown-linux-gnu
```

## ライセンス

Apache License 2.0

```text
Copyright 2026-2026, Wandbox LLC (Original Author)
Copyright 2026-2026, Shiguredo Inc.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```
