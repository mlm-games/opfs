# ropfs

[![Crates.io](https://img.shields.io/crates/v/ropfs)](https://crates.io/crates/ropfs)
[![License](https://img.shields.io/github/license/mlm-games/ropfs)](LICENSE)
[![docs.rs](https://img.shields.io/docsrs/ropfs)](https://docs.rs/ropfs)

Rust wrapper for the the [Origin Private File System](https://developer.mozilla.org/en-US/docs/Web/API/File_System_API/Origin_private_file_system) browser API. (This is an API that gives webapps limited access to the native file system.) 

This library mostly exists because using the OPFS from Rust is very painful. As a bonus, it also gives you support for native platforms for free - when compiling to native platforms, it will use `tokio::fs` instead of browser APIs.

## Overview

This crate provides an API for file system operations that automatically uses the appropriate implementation based on the target platform:

- **Web (WASM)**: Uses the Origin Private File System (OPFS) API
- **Native platforms**: Uses `tokio::fs`

An in-memory filesystem is also provided for use in tests (or when persistence isn't necessary)

## Features

- **Write once, run anywhere**: The same code works natively and on the web
- **Async/await**: All operations are asynchronous
- **Type safety**: The type-unsafe JsValue soup associated with working with browser APIs from Rust is hidden behind a type-safe API.

## Installation

```
cargo add ropfs
```

## Usage

```rust
use ropfs::persistent::{DirectoryHandle, FileHandle, WritableFileStream, app_specific_dir};
use ropfs::{GetFileHandleOptions, CreateWritableOptions};
use ropfs::persistent;

// you must import the traits to call methods on the types
use ropfs::{DirectoryHandle as _, FileHandle as _, WritableFileStream as _};

// This code works on both native and web platforms
async fn example(dir: DirectoryHandle) -> persistent::Result<()> {
    let options = GetFileHandleOptions { create: true };
    let mut file = dir.get_file_handle_with_options("example.txt", &options).await?;
    
    let write_options = CreateWritableOptions { keep_existing_data: false, mode: Default::default() };
    let mut writer = file.create_writable_with_options(&write_options).await?;
    
    writer.write_at_cursor_pos(b"Hello, world!").await?;
    writer.close().await?;
    
    let data = file.read().await?;
    println!("File contents: {:?}", String::from_utf8(data));
    
    Ok(())
}

async fn use_example() -> persistent::Result<()> {
    let directory: DirectoryHandle = app_specific_dir().await?;
    example(directory).await?;
    Ok(())
}
```

## High-level API

`ropfs::AppFs` wraps the app directory with path-based helpers so most code
never touches handles directly:

```rust
use ropfs::AppFs;

let mut app = AppFs::new().await.unwrap();
app.write("saves/slot1.bin", &data).await.unwrap();
app.append("log.txt", b"hit\n").await.unwrap();
let text = app.read_to_string("config.json").await.unwrap();
app.rename("saves/slot1.bin", "saves/slot1.bak").await.unwrap();
let estimate = app.estimate().await.unwrap(); // quota/usage
```

It also offers `metadata`/`is_file`/`is_dir`, `copy`, `remove_dir`/
`remove_dir_all`, `list_dir`/`list_recursive`, `clear`, and
`persist`/`persisted`. Multi-step operations (`rename`, `copy`) copy first
and delete last — they are not atomic.

## Storage location

`AppFs::new()` uses a legacy executable-stem directory
(`~/.local/share/<app-name>/` on Linux). New applications should prefer
`AppFs::new_for(&AppInfo { qualifier, organization, application })`, which
resolves OS-conventional project paths via the `directories` crate and
migrates legacy data forward once (non-destructively — the old directory is
left behind).

## Testing

```bash
cargo test                                        # host: native + memory backends
cargo test --target wasm32-unknown-unknown --lib  # headless Chromium: OPFS + memory
```

The wasm suite runs a shared conformance battery (`src/conformance.rs`)
against every backend. It needs `chromedriver` or `geckodriver` on `PATH`;
set `NO_HEADLESS=1` for a visible browser.

## Origin

Forked from [anchpop/opfs](https://github.com/anchpop/opfs), renamed to `ropfs`.

## Contributing

Issues and PRs are welcome, especially for:
- Correctness bugs
- Platform gaps (native vs web behaviour differences)

```bash
git clone https://github.com/mlm-games/ropfs
cd ropfs
cargo test
```

## License

MIT

See [LICENSE](LICENSE) for more info.
