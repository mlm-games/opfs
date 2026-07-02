# OPFS Rust

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
cargo add opfs
```

## Usage

```rust
use opfs::persistent::{DirectoryHandle, FileHandle, WritableFileStream, app_specific_dir};
use opfs::{GetFileHandleOptions, CreateWritableOptions};
use opfs::persistent;

// you must import the traits to call methods on the types
use opfs::{DirectoryHandle as _, FileHandle as _, WritableFileStream as _};

// This code works on both native and web platforms
async fn example(dir: DirectoryHandle) -> persistent::Result<()> {
    let options = GetFileHandleOptions { create: true };
    let mut file = dir.get_file_handle_with_options("example.txt", &options).await?;
    
    let write_options = CreateWritableOptions { keep_existing_data: false };
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
