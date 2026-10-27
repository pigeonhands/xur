# Xur

Xur is a small, expression-oriented programming language with Rust-like
syntax. Modules are ordinary values, types are checked at runtime where you
write them, and anything written in Rust can be made available to Xur
through [facet](https://facet.rs) reflection.

> **Status:** early development. Programs run: they are parsed, have their
> names resolved across modules, are compiled to bytecode and interpreted.
> A static type checker and a JIT are future work.

```console
$ cargo run -- run example/main.xur
Hello, World
...
```

There are complete programs in [`example/`](example):

| Example                                    | Shows                                             |
| ------------------------------------------ | ------------------------------------------------- |
| [`main.xur`](example/main.xur)             | most of the language, using `math.xur`            |
| [`channels.xur`](example/channels.xur)     | threads sending results back over a channel       |
| [`echo_server.xur`](example/echo_server.xur) | a TCP echo server, to try with `nc`             |
| [`bytes.xur`](example/bytes.xur)           | building and parsing a binary format with `bytes` |
| [`structs.xur`](example/structs.xur)       | structs and enum variants with named fields       |
| [`reflect.xur`](example/reflect.xur)       | listing fields and calling methods by name        |
| [`layout.xur`](example/layout.xur)         | structs as bytes: padding, packing, byte order    |
| [`re_build.xur`](example/re_build.xur)     | building a DLL, then reading it and an ELF back   |
| [`re_image.xur`](example/re_image.xur)     | reading an image by virtual address, PE or ELF    |

## A taste of Xur

### Modules are values

```rust
use std;

const math = std::import("./math.xur"); // load a module

fn main() {
    let io = std::io; // bind a module to a variable
    io::println("1 + 2 = {}", math.add(1, 2));
}
```

`std::import` is an ordinary function, so the path can be computed at
runtime and the module stored like any other value:

```rust
let modules = [math, std::import(plugin_path)];
for m in modules {
    io::println("2 + 3 = {}", m.add(2, 3));
}
```

### Types are checked where you write them

Xur is dynamically checked. A type you write is enforced when a value
arrives; leave it out, or write `_` or a type parameter, and anything goes.

```rust
fn scale(x: f64, by: u8) -> f64 { x * by.to_float() }

scale(2.0, 3);    // 6
scale(2.0, 300);  // runtime error: expected `u8`, found 300

fn first(items: _) { items[0] } // no checks at all
```

### Functions and generics

```rust
pub fn add(a: u32, b: u32) -> u32 {
    a + b // the last expression is the return value
}

fn print_name<T>() {
    std::io::println("Struct name: {}", std::any::type_name::<T>());
}

print_name::<ZeroPacked>();
```

### `if` and `match` are expressions, and `impl` adds methods

```rust
let parity = if i % 2 == 0 { "even" } else { "odd" };

enum Shape {
    Circle(f64)
    Rect(f64, f64)
}

impl Shape {
    // a method, called on a value: `shape.area()`
    pub fn area(self) -> f64 {
        match self {
            Shape::Circle(r) => 3.14159 * r * r,
            Shape::Rect(w, h) => w * h,
        }
    }

    // an associated function, called on the type: `Shape::square(2.0)`
    pub fn square(side: f64) -> Self {
        Shape::Rect(side, side)
    }
}
```

### Errors are values: `Result` and `?`

`Option` and `Result` are ordinary enums, with their methods (`unwrap`,
`map`, `is_ok`, ...) written in Xur in the [prelude](crates/xur-resolve/src/prelude.xur).

```rust
pub fn div(a: u32, b: u32) -> Result<u32, MathError> {
    if b == 0 {
        Err(MathError::DivideByZero)
    } else {
        Ok(a / b)
    }
}

// `?` returns early with the `Err`, otherwise unwraps the `Ok` value
fn average(total: u32, count: u32) -> Result<u32, MathError> {
    let avg = div(total, count)?;
    Ok(avg)
}

match average(10, 0) {
    Ok(avg) => io::println("average = {}", avg),
    Err(MathError::DivideByZero) => io::println("cannot average zero items"),
    Err(_) => io::println("something else went wrong"),
}
```

### Lists and closures

Lists are shared and change in place, like Python's. Closures capture the
values of the variables they use.

```rust
let xs = [1, 2, 3];
xs.push(4);
xs[0] = 10;

let offset = 10;
let shift = |x| x * 2 + offset;
io::println("{} {}", xs, shift(4)); // [10, 2, 3, 4] 18
```

### Structs

Structs are values, like tuples: assigning to a field gives the variable an
updated copy, and other copies keep their old value.

```rust
struct Point { x: i64, y: i64 }

let p = Point { x: 1, y: 2 };
let q = p;
p.x = 10;
let Point { x, .. } = p;
io::println("{} {} {}", p, q.x, x); // Point { x: 10, y: 2 } 1 10
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

### Control over memory layout

```rust
#[repr(C)] // C ABI alignment & padding
struct CPacked {
    a: usize
    b: u32
}

#[repr(align(4))]
#[endian(Little)]
struct Aligned {
    a: u32
    b: u32
    #[field(offset=100)] c: u32 // 100 bytes from the start of the struct
}

// every struct knows its bytes
let value = Aligned { a: 1, b: 2, c: 3 };
value.byte_size();                          // 104
let data = value.to_bytes();                // c's bytes are at 100
std::mem::from_bytes::<Aligned>(data);      // and back
std::mem::size_of::<Aligned>();             // 104

// layouts can also be inspected, or made at runtime
let layout = std::reflect::layout::<Aligned>(); // size 104, align 4
let data = layout.to_bytes(Aligned { a: 1, b: 2, c: 3 });
let back = layout.from_bytes(data);

// or a layout made at runtime
let entry = std::reflect::Layout::new("Entry").field("id", "u16").bytes("name", 8).packed();
```

## The standard library

`std` is written in Rust and registered like any other native module (see
below). Large modules are lazy: they are only built if a program uses them.

| Module        | Contents                                                                 |
| ------------- | ------------------------------------------------------------------------ |
| `std`         | `import(path)`, `panic(format, ...)`                                     |
| `std::io`     | `print`, `println`, with `{}` placeholders                               |
| `std::fs`     | `read_to_string`, `write`, `append`, `exists`, `remove`, `read_dir`, `File` |
| `std::net`    | `TcpListener`, `TcpStream`, `UdpSocket`, `UnixListener`, `UnixStream`    |
| `std::thread` | `spawn`, `sleep`, `JoinHandle::join`                                     |
| `std::sync`   | `channel`, `Sender::send`, `Receiver::recv`, `Receiver::try_recv`        |
| `std::math`   | `sqrt`, `pow`, `abs`, `min`, `max`                                       |
| `std::any`    | `type_name::<T>()`                                                       |
| `std::re`     | PE and ELF files with [pewter](https://docs.rs/pewter): `AnyImage`, `pe::File`, `pe::ImageDef`, `elf::File`, and every structure inside as typed records |
| `std::mem`    | `size_of::<T>()`, `align_of::<T>()`, `from_bytes::<T>(data)`, `to_bytes(v)` |
| `std::reflect`| `fields`, `get`, `set`, `methods`, `call` of any value; `layout::<T>()`, `Layout` |

Built-in values have methods too: lists (`len`, `push`, `pop`, `get`,
`insert`, `remove`, `contains`, `extend`, `join`, ...), strings (`len`,
`split`, `trim`, `to_upper`, `parse_int`, ...) and numbers (`abs`, `pow`,
`sqrt`, `round`, ...).

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
$ cargo run -- debug bytecode example/math.xur
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
mistake in a file is reported in one run:

```console
$ cargo run -- run bad.xur
Error: expected one of `(`, `.`, `;`, `?`, `[`, or an operator, found keyword `let`
   ╭─[ bad.xur:2:14 ]
   │
 2 │     let x = 1
   │              │
   │              ╰─ expected one of 6 possible tokens
 3 │     let y = x;
   │     ─┬─
   │      ╰─── unexpected token
───╯
error: could not compile `bad.xur` due to 1 error(s)
```

## Workspace layout

Executables live in top-level folders (`xur/`); the libraries they're built
from live in `crates/`, structured after rustc, whose crates these mirror:

| Crate                 | Purpose                                                       | rustc analogue  |
| --------------------- | ------------------------------------------------------------- | --------------- |
| `xur`                 | The `xur` command-line tool                                   | `rustc_driver`  |
| `crates/xur-span`     | Source map, spans and interned symbols                        | `rustc_span`    |
| `crates/xur-errors`   | Diagnostics and their rendering                               | `rustc_errors`  |
| `crates/xur-ast`      | Syntax tree types and visitor                                 | `rustc_ast`     |
| `crates/xur-parser`   | Lexer and recursive-descent parser                            | `rustc_parse`   |
| `crates/xur-resolve`  | Module loading, name resolution and the prelude               | `rustc_resolve` |
| `crates/xur-bytecode` | The register-based bytecode and lowering to it                | `rustc_mir_build` |
| `crates/xur-runtime`  | Values, the native registry and facet conversions             |                 |
| `crates/xur-vm`       | The bytecode interpreter                                      |                 |
| `crates/xur-jit`      | A Cranelift JIT for the bytecode (not implemented yet)        | `rustc_codegen_cranelift` |
| `crates/xur-std`      | The standard library, as native modules                       |                 |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## License

Licensed under either of MIT or Apache-2.0, at your option.
