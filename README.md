# Xur

A scripting language for security research — PoC development, binary
analysis, process inspection, and reverse engineering, with Rust-like
syntax and direct access to native tooling through
[facet](https://facet.rs) reflection.

[![crates.io](https://img.shields.io/crates/v/xur)](https://crates.io/crates/xur)
[![license](https://img.shields.io/crates/l/xur)](LICENSE-MIT)

> **Status:** early development. Programs run: they are parsed, have their
> names resolved across modules, are compiled to bytecode and interpreted.
> A static type checker and a JIT are future work.

## What it looks like

### Read a process's memory

```rust
use std::io;
use std::re::proc;

fn main() -> Result<()> {
    let handle = proc::open_current()?;
    let base = handle.modules()?[0].base;
    io::println("bytes at {}: {}", base.to_hex(), handle.read(base, 4)?.to_hex());
    Ok(())
}
```

### Decode x86 instructions

```rust
use std::io;
use std::re::asm::x86;

fn main() -> Result<()> {
    let bytes = std::bytes::from_list([0x90, 0x31, 0xC0, 0xC3]);
    let insns = x86::decode(64, bytes, 0x1000);

    for insn in insns {
        io::println("{}  {}  {}", insn.ip().to_hex(), insn, insn.mnemonic());
    }
    Ok(())
}
```

### Build a PE DLL

```rust
use std::io;
use std::re::pe;
use std::re::pe::ImageFileMachine;

fn main() -> Result<()> {
    let dll = pe::ImageDef::new_dll(ImageFileMachine::Amd64);
    let flags = pe::section_flags(["CNT_CODE", "MEM_EXECUTE", "MEM_READ"])?;
    dll.add_section(".text", flags, b"\xc3"); // a single `ret`
    let bytes = dll.write()?;
    io::println("built {} bytes, magic {}", bytes.len(), bytes.read_u16_le(0).to_hex());
    Ok(())
}
```

### Assemble, inject and execute remotely

```rust
use std::re::asm::x86;
use std::re::proc;

fn main() -> Result<()> {
    let mov = x86::insn("Mov_r32_imm32", "EAX", 42)?;
    let ret = x86::insn("Retnq")?;
    let code = x86::encode(64, [mov, ret], 0)?;

    let child = std::process::spawn("sleep", ["10"])?;
    let handle = proc::open(child.pid(), "readwrite")?;

    let address = handle.inject_bytes(code)?;
    let result = handle.execute_remote(address, 0)?;
    std::io::println("result: {}", result); // 42

    child.kill()?;
    child.wait()?;
    Ok(())
}
```

### Inspect a .NET assembly

```rust
use std::io;
use std::re::dotnet;

fn main() -> Result<()> {
    let loaded = dotnet::parse(dotnet::new("Example").write()?)?;
    for ty in loaded.all_types() {
        io::println("type {} ({})", ty.name, dotnet::type_attributes_names(ty.flags));
        for method in ty.methods {
            io::println("  method: {}", method.name);
        }
    }
    Ok(())
}
```

## The language

Xur has Rust-like syntax — `let`, `fn`, `struct`, `enum`, `impl`, `match`,
`if`/`else`, closures, generics — but is dynamically checked: types you
write are enforced when a value arrives, leave them out and anything goes.

### Modules are values

```rust
use std;

const math = std::import("./math.xur");

fn main() {
    let io = std::io;
    io::println("1 + 2 = {}", math.add(1, 2));
}
```

`std::import` is an ordinary function, so the path can be computed at
runtime and the module stored like any other value.

### Errors are values: `Result` and `?`

`Option` and `Result` are ordinary enums, with their methods (`unwrap`,
`map`, `is_ok`, ...) written in Xur in the
[prelude](crates/xur-std/prelude/). `?` returns early with the `Err` or
`None`, otherwise unwraps the value.

```rust
fn main() -> Result<(), str> {
    let config = std::fs::read_to_string("config.txt")?;
    std::io::println("{}", config);
    Ok(())
}
```

### Control over memory layout

Structs can specify C ABI layout, alignment, endianness and field offsets —
useful for parsing binary formats and interacting with native structures.

```rust
#[repr(C)]
#[endian(Little)]
struct Header {
    magic: u32
    #[field(offset=8)] payload_size: u32
}

let data = Header { magic: 0xdeadbeef, payload_size: 256 }.to_bytes();
let parsed = std::mem::from_bytes::<Header>(data);
```

Layouts can also be constructed at runtime from field specifications:

```rust
let entry = std::reflect::Layout::new("Entry").field("id", "u16").bytes("name", 8).packed();
```

### Threads, channels and sockets

```rust
let (sender, receiver) = std::sync::channel();
std::thread::spawn(|| sender.send(6 * 7).unwrap());
io::println("{}", receiver.recv().unwrap()); // 42

let listener = std::net::TcpListener::bind("127.0.0.1:7878").unwrap();
let (client, peer) = listener.accept().unwrap();
client.write(client.read_line().unwrap()).unwrap();
```

See the full set of language features in the
[`example/lang/`](example/lang) directory.

## The standard library

`std` is written in Rust and registered like any other native module. Large
modules are lazy: they are only built if a program uses them.

| Module         | Contents                                                                 |
| -------------- | ------------------------------------------------------------------------ |
| `std::re`      | binaries with [pewter](https://docs.rs/pewter) (`pe::PEFile`, `pe::ImageDef`, `elf::ElfFile`), live processes with [procodile](https://docs.rs/procodile) (`proc`), x86 disassembly/assembly with [iced-x86](https://docs.rs/iced-x86) (`asm::x86`), and .NET assemblies with [hao](https://docs.rs/hao) (`dotnet`) |
| `std::mem`     | `size_of::<T>()`, `align_of::<T>()`, `from_bytes::<T>(data)`, `to_bytes(v)` |
| `std::reflect` | `fields`, `get`, `set`, `methods`, `call` of any value; `layout::<T>()`, `Layout` |
| `std`          | `import(path)`, `panic(format, ...)`                                     |
| `std::io`      | `print`, `println`, with `{}` placeholders                               |
| `std::fs`      | `read_to_string`, `write`, `append`, `exists`, `remove`, `read_dir`, `File` |
| `std::net`     | `TcpListener`, `TcpStream`, `UdpSocket`, `UnixListener`, `UnixStream`    |
| `std::thread`  | `spawn`, `sleep`, `JoinHandle::join`                                     |
| `std::process` | `spawn(program, args)` an OS child process; `Process::pid/kill/wait`     |
| `std::sync`    | `channel`, `Sender::send`, `Receiver::recv`, `Receiver::try_recv`        |
| `std::math`    | `sqrt`, `pow`, `abs`, `min`, `max`                                       |
| `std::any`     | `type_name::<T>()`                                                       |

Built-in values have methods too: lists (`len`, `push`, `pop`, `get`,
`insert`, `remove`, `contains`, `extend`, `join`, ...), strings (`len`,
`split`, `trim`, `to_upper`, `parse_int`, ...) and numbers (`abs`, `pow`,
`sqrt`, `round`, ...).

## Examples

There are complete programs in [`example/`](example):

| Example | Shows |
| ------- | ----- |
| [`re/proc/read.xur`](example/re/proc/read.xur) | reading a process's memory by address |
| [`re/proc/modules.xur`](example/re/proc/modules.xur) | listing a process's loaded modules |
| [`re/proc/regions.xur`](example/re/proc/regions.xur) | walking mapped memory regions and protection flags |
| [`re/proc/inject.xur`](example/re/proc/inject.xur) | loading a shared library into another process |
| [`re/image/build.xur`](example/re/image/build.xur) | building a PE image in memory |
| [`re/image/fields.xur`](example/re/image/fields.xur) | reading PE header fields through facet |
| [`re/image/loaded.xur`](example/re/image/loaded.xur) | parsing a PE/ELF from this process |
| [`re/asm/decode.xur`](example/re/asm/decode.xur) | decoding x86-64 bytes into instructions |
| [`re/asm/encode.xur`](example/re/asm/encode.xur) | decoding and re-encoding instructions |
| [`re/asm/format.xur`](example/re/asm/format.xur) | formatting instructions with operand details |
| [`re/asm/inspect.xur`](example/re/asm/inspect.xur) | inspecting instruction properties |
| [`re/asm/jit.xur`](example/re/asm/jit.xur) | assembling, injecting, and executing code remotely |
| [`re/dotnet/dump.xur`](example/re/dotnet/dump.xur) | listing types and methods in a .NET assembly |
| [`re/dotnet/new.xur`](example/re/dotnet/new.xur) | creating a .NET assembly from scratch |
| [`re/dotnet/il.xur`](example/re/dotnet/il.xur) | emitting IL bytecode for a .NET method |
| [`re/dotnet/csharp.xur`](example/re/dotnet/csharp.xur) | generating C# source from a .NET assembly |
| [`re/dotnet/runtime.xur`](example/re/dotnet/runtime.xur) | reading a .NET runtime header |
| [`re/dotnet/il2cpp.xur`](example/re/dotnet/il2cpp.xur) | reading Unity Il2Cpp metadata |
| [`bytes/bytes.xur`](example/bytes/bytes.xur) | building and parsing a binary format with `bytes` |
| [`reflect/layout.xur`](example/reflect/layout.xur) | memory layouts: packed structs, endianness, runtime fields |
| [`reflect/reflect.xur`](example/reflect/reflect.xur) | listing fields, calling methods |
| [`reflect/endianness.xur`](example/reflect/endianness.xur) | big/little endian byte conversion |
| [`reflect/parse_bytes.xur`](example/reflect/parse_bytes.xur) | parsing bytes into structs with layouts |
| [`reflect/func.xur`](example/reflect/func.xur) | function reflection |
| [`reflect/types_as_values.xur`](example/reflect/types_as_values.xur) | types as first-class values |
| [`mem/sizes.xur`](example/mem/sizes.xur) | struct sizes and alignment |
| [`mem/to_bytes.xur`](example/mem/to_bytes.xur) | struct to/from bytes |
| [`lang/main.xur`](example/lang/main.xur) | most of the language, using `math.xur` |
| [`lang/structs.xur`](example/lang/structs.xur) | structs and enum variants with named fields |
| [`lang/types.xur`](example/lang/types.xur) | generics, `T!`, `@` record mode, type inference |
| [`lang/coerce.xur`](example/lang/coerce.xur) | struct coercion with `as` and `try_cast` |
| [`lang/patterns.xur`](example/lang/patterns.xur) | pattern matching |
| [`lang/maps.xur`](example/lang/maps.xur) | hash maps |
| [`lang/records.xur`](example/lang/records.xur) | anonymous records |
| [`lang/fn_types.xur`](example/lang/fn_types.xur) | function types and higher-order functions |
| [`lang/decorators.xur`](example/lang/decorators.xur) | decorator syntax |
| [`json/`](example/json) | JSON parsing and struct coercion |
| [`cli/`](example/cli) | argument parsing, prompts, subcommands, editors |
| [`sync/channels.xur`](example/sync/channels.xur) | threads sending results back over a channel |
| [`net/echo_server.xur`](example/net/echo_server.xur) | a TCP echo server |
| [`module/`](example/module) | dynamic imports and module introspection |
| [`stub/game_inspector.xur`](example/stub/game_inspector.xur) | in-process Unity Il2Cpp game inspector via `LD_PRELOAD` |
| [`stub/egui_overlay.xur`](example/stub/egui_overlay.xur) | egui overlay injected into a running process |

## Extending Xur with Rust

Any Rust function whose argument and return types implement `Facet` can be
registered, and any `#[derive(Facet)]` type becomes a Xur type whose values
are the Rust values themselves:

```rust
use facet::Facet;

#[derive(Facet, Clone)]
struct Point { x: f64, y: f64 }

let registry = xur_std::registry();
registry
    .module("geo")
    .function("distance", |a: Point, b: Point| (a.x - b.x).hypot(a.y - b.y))
    .ty::<Point>("Point")
    .function("new", |x: f64, y: f64| Point { x, y }) // Point::new(1.0, 2.0)
    .method("norm", |p: &Point| p.x.hypot(p.y)) //     p.norm()
    .method_mut("scale", |p: &mut Point, k: f64| { p.x *= k; p.y *= k }); // in place
```

Arguments are converted from Xur values (range-checking integers) and
results back: `Option` and `Result` become the prelude's enums, `Vec`s
become lists, and so on. A module can be registered lazily with
`registry.lazy_module(path, |m| ...)`, and functions that need the running
program, like `std::import`, take a `Context` instead.

## How it works

```text
source ─▶ lexer ─▶ parser ─▶ AST ─▶ name resolution ─▶ bytecode ─▶ VM
                                    (follows imports)   (lowered on first call)
```

The bytecode is a control-flow graph of basic blocks over virtual
registers, shaped like Cranelift's IR so that a JIT can later compile the
same code the interpreter runs; see [ARCHITECTURE.md](ARCHITECTURE.md) for
the whole journey. `xur debug` shows each stage:

```console
$ xur debug bytecode example/lang/math.xur
fn div (captures: 0, params: 2, registers: 9)
  bb0:
    check r0: u32
    check r1: u32
    r2 = 0
    r3 = r1 == r2
    branch r3 ? bb1 : bb2
  ...
```

| Command                    | Shows                                                    |
| -------------------------- | -------------------------------------------------------- |
| `xur run <file>`           | runs the program's `main`                                |
| `xur debug tokens <file>`  | the tokens of one file                                   |
| `xur debug ast <file>`     | the syntax tree of one file                              |
| `xur debug resolve <file>` | what every name in the program refers to                 |
| `xur debug bytecode <file>`| the bytecode of every function in one file               |

Errors, at compile time or at runtime, are shown as annotated source
snippets. Like rustc, the parser and resolver recover from errors, so every
mistake in a file is reported in one run.

## Crates

| | Crate | Purpose |
|-|-------|---------|
| [![crates.io](https://img.shields.io/crates/v/xur)](https://crates.io/crates/xur) | [`xur`](xur) | CLI tool — `xur run`, `xur debug`, `xur lsp` |
| [![crates.io](https://img.shields.io/crates/v/xur-stub)](https://crates.io/crates/xur-stub) | [`xur-stub`](xur-stub) | `LD_PRELOAD` stub for in-process injection |
| [![crates.io](https://img.shields.io/crates/v/xur-span)](https://crates.io/crates/xur-span) | [`xur-span`](crates/xur-span) | Source map, spans and interned symbols |
| [![crates.io](https://img.shields.io/crates/v/xur-errors)](https://crates.io/crates/xur-errors) | [`xur-errors`](crates/xur-errors) | Diagnostics and rendered error snippets |
| [![crates.io](https://img.shields.io/crates/v/xur-ast)](https://crates.io/crates/xur-ast) | [`xur-ast`](crates/xur-ast) | Syntax tree types and visitor |
| [![crates.io](https://img.shields.io/crates/v/xur-parser)](https://crates.io/crates/xur-parser) | [`xur-parser`](crates/xur-parser) | Lexer and recursive-descent parser |
| [![crates.io](https://img.shields.io/crates/v/xur-resolve)](https://crates.io/crates/xur-resolve) | [`xur-resolve`](crates/xur-resolve) | Module loading and name resolution |
| [![crates.io](https://img.shields.io/crates/v/xur-bytecode)](https://crates.io/crates/xur-bytecode) | [`xur-bytecode`](crates/xur-bytecode) | Register-based bytecode IR and lowering |
| [![crates.io](https://img.shields.io/crates/v/xur-runtime)](https://crates.io/crates/xur-runtime) | [`xur-runtime`](crates/xur-runtime) | Values, native registry and facet conversions |
| [![crates.io](https://img.shields.io/crates/v/xur-vm)](https://crates.io/crates/xur-vm) | [`xur-vm`](crates/xur-vm) | Bytecode interpreter |
| [![crates.io](https://img.shields.io/crates/v/xur-jit)](https://crates.io/crates/xur-jit) | [`xur-jit`](crates/xur-jit) | Cranelift JIT (not yet implemented) |
| [![crates.io](https://img.shields.io/crates/v/xur-std)](https://crates.io/crates/xur-std) | [`xur-std`](crates/xur-std) | Standard library, prelude and native modules |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## License

Licensed under either of MIT or Apache-2.0, at your option.
