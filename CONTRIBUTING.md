# Contributing

## Local setup

Install stable Rust 1.85+, the Windows MSVC Rust target, and Visual Studio Build Tools with the C++ workload and Windows SDK. A Vulkan-capable graphics driver is required to launch the app. Install the Vulkan SDK when you need Khronos validation or shader compilation.

## Development commands

```powershell
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo run -p project3-6k-app
```

Keep crate responsibilities explicit, document Vulkan unsafe assumptions, and update the architecture docs when ownership or dependency directions change. Do not add empty crates or gameplay code during engine-foundation work.
