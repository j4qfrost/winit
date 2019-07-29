# This branch has been archived

Please direct all work against `master`.

--------

# winit - Cross-platform window creation and management in Rust

[![Crates.io](https://img.shields.io/crates/v/winit.svg)](https://crates.io/crates/winit)
[![Docs.rs](https://docs.rs/winit/badge.svg)](https://docs.rs/winit)
[![Build Status](https://travis-ci.org/rust-windowing/winit.svg?branch=master)](https://travis-ci.org/rust-windowing/winit)
[![Build status](https://ci.appveyor.com/api/projects/status/hr89but4x1n3dphq/branch/master?svg=true)](https://ci.appveyor.com/project/Osspial/winit/branch/master)

```toml
[dependencies]
winit = "0.19.2"
```

## [Documentation](https://docs.rs/winit)

For features _within_ the scope of winit, see [FEATURES.md](FEATURES.md).

For features _outside_ the scope of winit, see [Missing features provided by other crates](https://github.com/rust-windowing/winit/wiki/Missing-features-provided-by-other-crates) in the wiki.

## Contact Us

Join us in any of these:

[![Freenode](https://img.shields.io/badge/freenode.net-%23glutin-red.svg)](http://webchat.freenode.net?channels=%23glutin&uio=MTY9dHJ1ZSYyPXRydWUmND10cnVlJjExPTE4NSYxMj10cnVlJjE1PXRydWU7a)
[![Matrix](https://img.shields.io/badge/Matrix-%23Glutin%3Amatrix.org-blueviolet.svg)](https://matrix.to/#/#Glutin:matrix.org)
[![Gitter](https://badges.gitter.im/Join%20Chat.svg)](https://gitter.im/tomaka/glutin?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)

## Usage

Winit is a window creation and management library. It can create windows and lets you handle
events (for example: the window being resized, a key being pressed, a mouse movement, etc.)
produced by window.

Winit is designed to be a low-level brick in a hierarchy of libraries. Consequently, in order to
show something on the window you need to use the platform-specific getters provided by winit, or
another library.

```rust
extern crate winit;

fn main() {
    let mut events_loop = winit::EventsLoop::new();
    let window = winit::Window::new(&events_loop).unwrap();

    events_loop.run_forever(|event| {
        match event {
            winit::Event::WindowEvent {
              event: winit::WindowEvent::CloseRequested,
              ..
            } => winit::ControlFlow::Break,
            _ => winit::ControlFlow::Continue,
        }
    });
}
```

Winit is only officially supported on the latest stable version of the Rust compiler.

### Cargo Features

Winit provides the following features, which can be enabled in your `Cargo.toml` file:
* `icon_loading`: Enables loading window icons directly from files. Depends on the [`image` crate](https://crates.io/crates/image).
* `serde`: Enables serialization/deserialization of certain types with [Serde](https://crates.io/crates/serde).

### Platform-specific usage

#### Emscripten and WebAssembly

Building a binary will yield a `.js` file. In order to use it in an HTML file, you need to:

- Put a `<canvas id="my_id"></canvas>` element somewhere. A canvas corresponds to a winit "window".
- Write a Javascript code that creates a global variable named `Module`. Set `Module.canvas` to
  the element of the `<canvas>` element (in the example you would retrieve it via `document.getElementById("my_id")`).
  More information [here](https://kripken.github.io/emscripten-site/docs/api_reference/module.html).
- Make sure that you insert the `.js` file generated by Rust after the `Module` variable is created.
