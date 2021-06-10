# pydeno_core

This project provides Python bindings to the [deno_core][] runtime (v8 under
the hood) that powers [Deno][]. The goal is to facilitate running potentially
untrusted JavaScript easily from Python. Easily write high-level callbacks in
Python that can be used from JavaScript.

## Installation

pydeno_core is available from PyPi with prebuilt binaries:

    pip install pydeno_core

## Safety

All of the projects involved in this are relatively new. While deno is built on
the rock-solid v8, deno itself and this library aren't so battle-tested. Use at
your own risk.

The primary consumer of this library (Notifico) further isolates these
processes using [Firecracker][] when running in production.

## Performance

These bindings exist to enable rapid prototyping and iteration. Performance is
not one of the key goals, but a TODO. That said, the project will happily merge
performance PRs that don't significantly impact ease-of-use.

## Development

To build the project, first install [Rust][] and [Cargo][] with [rustup][].
Then you'll need to install the Python dependencies with pip:

    pip install -e ".[dev]"

This will install Maturin, pytest, and any other dependencies. You can now use
`maturin develop` to build the binding and link the `deno_core` module into
your current Python `virtualenv`.

To run the tests, just run `pytest`. Note the Rust side of this project is
tested from Python and doesn't have the typical rust tests.

If you're working on the deno_core side, be prepared to read the source and
don't trust the documentation. deno_core moves quickly, and the documentation
is rarely updated.

[Rust]: https://www.rust-lang.org/
[Cargo]: https://crates.io/
[rustup]: https://rustup.rs/
[firecracker]: https://firecracker-microvm.github.io/
[deno_core]: https://crates.io/crates/deno_core
[deno]: https://deno.land/
