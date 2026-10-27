use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use xur_resolve::Program;
use xur_span::MemoryFileLoader;

use crate::Vm;

/// Captures what a program prints.
#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Runs the program made of `files` (the first is the root), returning
/// what it printed, or the runtime error.
fn run_files(files: &[(&str, &str)]) -> Result<String, String> {
    let registry = Arc::new(xur_std::registry());
    let loader = MemoryFileLoader::new(files.iter().copied());
    let (program, diags) = Program::load(
        Path::new(files[0].0),
        Box::new(loader),
        Box::new(Arc::clone(&registry)),
    )
    .unwrap();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(diags.is_empty(), "{messages:?}");
    let output = Output::default();
    let vm = Vm::new(program, registry, Box::new(output.clone()));
    let result = vm.run_main();
    let printed = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
    result.map(|_| printed).map_err(|error| error.message)
}

fn run(src: &str) -> Result<String, String> {
    run_files(&[("main.xur", src)])
}

#[test]
fn hello_world() {
    assert_eq!(
        run("fn main() { std::io::println(\"Hello, {}!\", \"World\"); }"),
        Ok("Hello, World!\n".into())
    );
}

#[test]
fn arithmetic_control_flow_and_recursion() {
    let src = "
        fn fib(n: i64) -> i64 { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
        fn main() {
            let parity = if 7 % 2 == 0 { \"even\" } else { \"odd\" };
            std::io::println(\"{} {} {}\", fib(15), parity, 1.5 * 2.0);
        }";
    assert_eq!(run(src), Ok("610 odd 3\n".into()));
}

#[test]
fn enums_impls_and_match() {
    let src = "
        enum Shape { Circle(f64) Rect(f64, f64) }
        impl Shape {
            fn area(self) -> f64 { match self { Shape::Circle(r) => 3.0 * r * r, Shape::Rect(w, h) => w * h } }
            fn square(side: f64) -> Self { Shape::Rect(side, side) }
        }
        fn main() {
            let shapes = [Shape::Circle(1.0), Shape::square(2.0)];
            for s in shapes { std::io::println(\"{} {}\", s, s.area()); }
        }";
    assert_eq!(run(src), Ok("Circle(1) 3\nRect(2, 2) 4\n".into()));
}

#[test]
fn closures_capture_values() {
    let src = "
        fn apply(f: _, x: i64) -> i64 { f(x) }
        fn main() {
            let offset = 10;
            let shift = |x| { let y = x * 2; y + offset };
            std::io::println(\"{}\", apply(shift, 4));
        }";
    assert_eq!(run(src), Ok("18\n".into()));
}

#[test]
fn lists_are_shared_and_mutable() {
    let src = "
        fn main() {
            let xs = [1, 2, 3];
            let alias = xs;
            alias.push(4);
            xs[0] = 10;
            let total = 0;
            for x in xs { total = total + x; }
            std::io::println(\"{} {} {}\", xs, xs.len(), total);
        }";
    assert_eq!(run(src), Ok("[10, 2, 3, 4] 4 19\n".into()));
}

#[test]
fn results_and_the_try_operator() {
    let src = "
        fn half(n: i64) -> Result<i64, str> { if n % 2 == 0 { Ok(n / 2) } else { Err(\"odd\") } }
        fn quarter(n: i64) -> Result<i64, str> { let h = half(n)?; half(h) }
        fn main() {
            std::io::println(\"{} {}\", quarter(8), quarter(6));
            match \"12\".parse_int() { Ok(n) => std::io::println(\"{}\", n + 1), Err(e) => std::io::println(\"{}\", e) }
        }";
    assert_eq!(run(src), Ok("Ok(2) Err(\"odd\")\n13\n".into()));
}

#[test]
fn loops_break_and_continue() {
    let src = "
        fn main() {
            let found = loop { break 42; };
            let n = 0;
            while true { n = n + 1; if n < 5 { continue; } break; }
            std::io::println(\"{} {}\", found, n);
        }";
    assert_eq!(run(src), Ok("42 5\n".into()));
}

#[test]
fn modules_are_values_and_imports_can_be_dynamic() {
    let files = [
        (
            "main.xur",
            "const math = std::import(\"math.xur\");
             fn main() {
                 let name = \"math.xur\";
                 let modules = [math, std::import(name)];
                 for m in modules { std::io::println(\"{}\", m.add(2, 3)); }
                 std::io::println(\"{}\", math::add(1, 1));
             }",
        ),
        ("math.xur", "pub fn add(a: i64, b: i64) -> i64 { a + b }"),
    ];
    assert_eq!(run_files(&files), Ok("5\n5\n2\n".into()));
}

#[test]
fn type_names_follow_type_arguments() {
    let src = "
        struct Marker;
        fn name_of<T>() -> str { std::any::type_name::<T>() }
        fn main() { std::io::println(\"{} {}\", name_of::<Marker>(), name_of::<u32>()); }";
    assert_eq!(run(src), Ok("Marker u32\n".into()));
}

#[test]
fn runtime_errors() {
    assert_eq!(
        run("fn main() { 1 / 0; }"),
        Err("attempt to divide by zero".into())
    );
    assert_eq!(
        run("fn main() { 1 + 1.0; }"),
        Err("cannot apply `+` to integer and float".into())
    );
    assert_eq!(
        run("fn main() { [1].nope(); }"),
        Err("no method `nope` on list".into())
    );
    assert_eq!(
        run("fn f() {}"),
        Err("the program has no `main` function".into())
    );
}

#[test]
fn the_examples_run() {
    let output = run_files(&[
        (
            "example/lang/main.xur",
            include_str!("../../../example/lang/main.xur"),
        ),
        (
            "example/lang/math.xur",
            include_str!("../../../example/lang/math.xur"),
        ),
    ]);
    insta_like(
        output,
        "Hello, World
1 + 0 = 1 (even)
1 + 1 = 2 (odd)
1 + 2 = 3 (even)
1 + 3 = 4 (odd)
1 + 4 = 5 (even)
1 + 5 = 6 (odd)
1 + 6 = 7 (even)
1 + 7 = 8 (odd)
1 + 8 = 9 (even)
1 + 9 = 10 (odd)
shift(4) = 18
cannot average zero items
2 + 3 = 5
2 + 3 = 5
area = 7
square area = 4
Struct name: ZeroPacked
CPacked: 16 bytes, 01000000000000000200000000000000
ZeroPacked: 8 bytes
Aligned: 104 bytes, c = 3
back again: Aligned { a: 1, b: 2, c: 3 }
",
    );
}

fn insta_like(actual: Result<String, String>, expected: &str) {
    match actual {
        Ok(actual) if actual == expected => {}
        other => panic!("expected:\n{expected}\nactual:\n{other:?}"),
    }
}

#[test]
fn option_and_result_methods_are_written_in_xur() {
    let src = "
        fn main() {
            let some = Some(2);
            let none: Option<i64> = None;
            std::io::println(\"{} {} {}\", some.is_some(), none.unwrap_or(7), some.map(|x| x * 10));
            let parsed = \"x\".parse_int();
            std::io::println(\"{} {}\", parsed.is_err(), parsed.ok());
        }";
    assert_eq!(run(src), Ok("true 7 Some(20)\ntrue None\n".into()));
}

#[test]
fn panics_stop_the_program() {
    assert_eq!(
        run("fn main() { let x: Option<i64> = None; x.unwrap(); }"),
        Err("called `unwrap` on `None`".into())
    );
    assert_eq!(
        run("fn main() { std::panic(\"bad {}\", 1); }"),
        Err("bad 1".into())
    );
}

#[test]
fn files() {
    let dir = std::env::temp_dir().join(format!("xur-fs-test-{}", std::process::id()));
    let dir = dir.display().to_string();
    let src = format!(
        "fn main() {{
            let dir = \"{dir}\";
            std::fs::create_dir_all(dir).unwrap();
            let path = dir + \"/notes.txt\";
            std::fs::write(path, \"one\\n\").unwrap();
            std::fs::append(path, \"two\\n\").unwrap();
            let file = std::fs::File::open(path).unwrap();
            std::io::println(\"{{}}\", file.read_to_string().unwrap().lines());
            std::io::println(\"{{}} {{}}\", std::fs::exists(path), std::fs::read_dir(dir).unwrap());
            std::fs::write_bytes(path, b\"\\x00\\xff\").unwrap();
            std::io::println(\"{{}}\", std::fs::read(path).unwrap());
            std::fs::remove(path).unwrap();
            std::io::println(\"{{}}\", std::fs::read_to_string(path).is_err());
        }}"
    );
    let output = run(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        output,
        Ok("[\"one\", \"two\"]\ntrue [\"notes.txt\"]\nb\"\\x00\\xff\"\ntrue\n".into())
    );
}

#[test]
fn threads_and_channels() {
    let src = "
        fn main() {
            let (sender, receiver) = std::sync::channel();
            let workers = [];
            for i in 1..4 {
                workers.push(std::thread::spawn(|| {
                    sender.send(i * i).unwrap();
                    i
                }));
            }
            let joined = 0;
            for worker in workers { joined = joined + worker.join(); }
            let received = 0;
            for _ in 0..3 { received = received + receiver.recv().unwrap(); }
            std::io::println(\"{} {} {}\", joined, received, receiver.try_recv());
        }";
    assert_eq!(run(src), Ok("6 14 None\n".into()));
}

#[test]
fn errors_in_threads_reach_the_joiner() {
    let src = "fn main() { std::thread::spawn(|| 1 / 0).join(); }";
    assert_eq!(run(src), Err("attempt to divide by zero".into()));
}

#[test]
fn tcp_and_udp_sockets() {
    let src = "
        fn main() {
            let listener = std::net::TcpListener::bind(\"127.0.0.1:0\").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(|| {
                let (client, _) = listener.accept().unwrap();
                let line = client.read_line().unwrap();
                client.write(\"echo: \" + line).unwrap();
            });
            let stream = std::net::TcpStream::connect(address).unwrap();
            stream.write(\"hi\\n\").unwrap();
            std::io::print(\"{}\", stream.read_line().unwrap());
            server.join();

            let a = std::net::UdpSocket::bind(\"127.0.0.1:0\").unwrap();
            let b = std::net::UdpSocket::bind(\"127.0.0.1:0\").unwrap();
            a.send_to(\"ping\", b.local_addr().unwrap()).unwrap();
            let (text, from) = b.recv_from(64).unwrap();
            std::io::println(\"{} {}\", text, from == a.local_addr().unwrap());
            std::io::println(\"{}\", std::net::TcpStream::connect(\"256.0.0.1:1\").is_err());
        }";
    assert_eq!(run(src), Ok("echo: hi\nping true\ntrue\n".into()));
}

#[cfg(unix)]
#[test]
fn unix_sockets() {
    let path = std::env::temp_dir().join(format!("xur-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let src = format!(
        "fn main() {{
            let listener = std::net::UnixListener::bind(\"{}\").unwrap();
            let server = std::thread::spawn(|| {{ listener.accept().unwrap().write(\"over unix\\n\").unwrap(); }});
            let stream = std::net::UnixStream::connect(\"{}\").unwrap();
            std::io::print(\"{{}}\", stream.read_line().unwrap());
            server.join();
        }}",
        path.display(),
        path.display()
    );
    let output = run(&src);
    let _ = std::fs::remove_file(&path);
    assert_eq!(output, Ok("over unix\n".into()));
}

#[test]
fn the_channels_example_counts_primes() {
    let output = run_files(&[(
        "example/sync/channels.xur",
        include_str!("../../../example/sync/channels.xur"),
    )])
    .unwrap();
    let mut lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.pop(), Some("1229 primes below 10000"));
    lines.sort();
    assert_eq!(
        lines,
        [
            "0..2500: 367 primes",
            "2500..5000: 302 primes",
            "5000..7500: 281 primes",
            "7500..10000: 279 primes"
        ]
    );
}

#[test]
fn the_echo_server_example_compiles() {
    // It serves forever, so only check that it loads without errors.
    let registry = Arc::new(xur_std::registry());
    let src = include_str!("../../../example/net/echo_server.xur");
    let loader = MemoryFileLoader::new([("example/net/echo_server.xur", src)]);
    let (_, diags) = Program::load(
        Path::new("example/net/echo_server.xur"),
        Box::new(loader),
        Box::new(registry),
    )
    .unwrap();
    assert!(diags.is_empty(), "{diags:#?}");
}

#[test]
fn written_types_are_checked_at_runtime() {
    let check = |body: &str| run(&format!("struct Meters(f64);\nenum E {{ A }}\n{body}"));
    assert_eq!(
        check("fn f(x: u8) {} fn main() { f(300); }"),
        Err("expected `u8`, found 300".into())
    );
    assert_eq!(
        check("fn f(x: str) {} fn main() { f(1); }"),
        Err("expected `str`, found integer".into())
    );
    assert_eq!(
        check("fn main() { let m: Meters = E::A; }"),
        Err("expected `Meters`, found `E`".into())
    );
    assert_eq!(
        check("fn f() -> i64 { return \"no\"; } fn main() { f(); }"),
        Err("expected `i64`, found string".into())
    );
    assert_eq!(
        check("fn f(t: (i64, _)) {} fn main() { f((1, \"anything\")); f((1, 2, 3)); }"),
        Err("expected `(i64, _)`, found tuple".into())
    );
}

#[test]
fn unwritten_and_generic_types_are_not_checked() {
    let src = "fn id<T>(x: T) -> T { x }\nfn any(x: _) {}\nfn main() { id(1); id(\"a\"); any([]); let l = |x| x; l(1); }";
    assert_eq!(run(src), Ok(String::new()));
}

#[test]
fn bytes_are_shared_mutable_buffers() {
    let src = "
        fn main() {
            let header = b\"MZ\\x90\\x00\";
            let alias = header;
            alias[2] = 0x41;
            let typed: bytes = header + b\"!\";
            std::io::println(\"{} {} {} {}\", header, header[0], header[1..3], typed);
            for byte in b\"ab\" { std::io::print(\"{} \", byte); }
            std::io::println(\"{}\", b\"x\" == b\"x\");
        }";
    assert_eq!(
        run(src),
        Ok("b\"MZA\\x00\" 77 b\"ZA\" b\"MZA\\x00!\"\n97 98 true\n".into())
    );
    assert_eq!(
        run("fn main() { let b = b\"a\"; b[0] = 256; }"),
        Err("256 is not a byte (0 to 255)".into())
    );
}

#[test]
fn ranges_slice_lists_strings_and_tuples() {
    let src = "fn main() { std::io::println(\"{} {} {} {}\", [1, 2, 3, 4][1..], [1, 2, 3][..=1], \"héllo\"[1..3], (1, 2, 3)[1..]); }";
    assert_eq!(run(src), Ok("[2, 3, 4] [1, 2] él (2, 3)\n".into()));
    assert_eq!(
        run("fn main() { [1, 2][1..5]; }"),
        Err("range 1..5 is out of range for length 2".into())
    );
}

#[test]
fn bytes_methods() {
    let src = "
        fn main() {
            let data = std::bytes::zeroed(8);
            data.write_u32_le(0, 0xdeadbeef);
            data.write_i16_be(4, -2);
            std::io::println(\"{} {} {} {}\", data.to_hex(), data.read_u32_le(0).to_hex(), data.read_i16_be(4), data.read_u8(7));
            std::io::println(\"{} {}\", std::bytes::from_hex(\"4d5a\").unwrap(), \"hi\".to_bytes().decode());
            std::io::println(\"{}\", data.find(b\"\\xbe\\xad\"));
            std::io::println(\"{} {}\", [0x4d, 0x5a].to_hex(), \"4d 5a\".from_hex());
        }";
    assert_eq!(
        run(src),
        Ok(
            "efbeaddefffe0000 deadbeef -2 0\nb\"MZ\" Ok(\"hi\")\nSome(1)\n4d5a Ok(b\"MZ\")\n"
                .into()
        )
    );
    assert_eq!(
        run("fn main() { b\"ab\".read_u32_le(0); }"),
        Err("4 byte(s) at offset 0 are out of range for 2 byte(s)".into())
    );
    assert_eq!(
        run("fn main() { std::bytes::zeroed(1).write_u8(0, 256); }"),
        Err("256 does not fit in `u8`".into())
    );
}

#[test]
fn the_bytes_example_runs() {
    let output = run_files(&[(
        "example/bytes/bytes.xur",
        include_str!("../../../example/bytes/bytes.xur"),
    )]);
    assert_eq!(
        output,
        Ok("00000000  58 55 52 21 01 00 03 00 ad de 00 00 ef be 00 00\n00000010  2a 00 00 00\nentries: Ok([57005, 48879, 1337])\nErr(\"not a XUR! file\")\nmagic at 1\n".into())
    );
}

#[test]
fn structs_have_named_fields() {
    let src = r#"
        struct Point { x: i64, y: i64 }
        enum Shape { Circle { r: f64 }, Rect { w: f64, h: f64 } }
        fn area(s: Shape) -> f64 {
            match s { Shape::Circle { r } => 3.0 * r * r, Shape::Rect { w, h: height } => w * height }
        }
        fn main() {
            let y = 2;
            let p = Point { y, x: 1 };
            let q = p;
            p.x = 10;
            let Point { x, .. } = p;
            std::io::println("{} {} {} {}", p, q.x, x, area(Shape::Rect { w: 2.0, h: 3.0 }));
        }
    "#;
    assert_eq!(run(src), Ok("Point { x: 10, y: 2 } 1 10 6\n".into()));
}

#[test]
fn nested_fields_are_assigned_through_copies() {
    let src = r#"
        struct Inner { n: i64 }
        struct Outer { inner: Inner, items: [i64] }
        fn main() {
            let o = Outer { inner: Inner { n: 1 }, items: [1] };
            o.inner.n = 5;
            o.items[0] = 7;
            std::io::println("{}", o);
        }
    "#;
    assert_eq!(
        run(src),
        Ok("Outer { inner: Inner { n: 5 }, items: [7] }\n".into())
    );
}

#[test]
fn struct_literal_errors() {
    let with = |body: &str| {
        run(&format!(
            "struct P {{ x: u8, y: u8 }} fn main() {{ {body} }}"
        ))
    };
    assert_eq!(with("P { x: 1 };"), Err("missing field(s) `y`".into()));
    assert_eq!(
        with("P { x: 1, y: 2, z: 3 };"),
        Err("no field `z` in this struct".into())
    );
    assert_eq!(
        with("P { x: 1, x: 2, y: 3 };"),
        Err("field `x` specified more than once".into())
    );
    assert_eq!(
        with("P { x: 300, y: 0 };"),
        Err("expected `u8`, found 300".into())
    );
    assert_eq!(
        with("let p = P { x: 1, y: 2 }; p.z;"),
        Err("no field `z` on `P`".into())
    );
    assert_eq!(
        with("let P { x } = P { x: 1, y: 2 };"),
        Err("pattern does not mention field(s) `y`".into())
    );
}

#[test]
fn the_structs_example_runs() {
    let output = run_files(&[(
        "example/lang/structs.xur",
        include_str!("../../../example/lang/structs.xur"),
    )]);
    assert_eq!(
        output,
        Ok(concat!(
            "Player { name: \"ada\", pos: Point { x: 0, y: 0 }, health: 100 }\n",
            "Moved { dx: 3, dy: -1 } -> ada at (3, -1), health 100\n",
            "Hit { damage: 30 } -> ada at (3, -1), health 70\n",
            "Moved { dx: 1, dy: 1 } -> ada at (4, 0), health 70\n",
            "start is still at Point { x: 0, y: 0 }\n",
        )
        .into())
    );
}

#[test]
fn reflection_lists_reads_and_calls() {
    let src = r#"
        use std::reflect;
        struct Point { x: i64, y: i64 }
        enum Shape { Circle(f64), Rect { w: f64, h: f64 } }
        impl Point {
            pub fn origin() -> Self { Point { x: 0, y: 0 } }
            pub fn sum(self) -> i64 { self.x + self.y }
            pub fn scaled(self, k: i64) -> Self { Point { x: self.x * k, y: self.y * k } }
        }
        fn main() {
            let p = Point { x: 1, y: 2 };
            let io = std::io;
            io::println("{} {} {}", reflect::type_name(p), reflect::fields(p), reflect::methods(p));
            io::println("{} {} {}", reflect::get(p, "y"), reflect::set(p, "x", 9), reflect::call(p, "scaled", [3]));
            io::println("{} {}", reflect::fields(Shape::Rect { w: 1.0, h: 2.0 }), reflect::type_name(Shape::Circle(1.0)));
            io::println("{} {} {}", reflect::fields((1, 2)), reflect::get((1, 2), "1"), reflect::has_method([1], "push"));
            io::println("{} {}", reflect::try_get(p, "z"), reflect::call("a,b", "split", [","]));
            let (sender, receiver) = std::sync::channel();
            io::println("{} {}", reflect::type_name(sender), reflect::methods(sender));
            reflect::call(sender, "send", [5]);
            io::println("{}", receiver.recv());
        }
    "#;
    assert_eq!(
        run(src),
        Ok(concat!(
            "Point [\"x\", \"y\"] [\"sum\", \"scaled\"]\n",
            "2 Point { x: 9, y: 2 } Point { x: 3, y: 6 }\n",
            "[\"w\", \"h\"] Shape::Circle\n",
            "[\"0\", \"1\"] 2 true\n",
            "None [\"a\", \"b\"]\n",
            "Sender [\"send\"]\n",
            "Some(5)\n",
        )
        .into())
    );
    assert_eq!(
        run("fn main() { std::reflect::call(1, \"nope\", []); }"),
        Err("no method `nope` on integer".into())
    );
}

#[test]
fn the_reflect_example_runs() {
    let output = run_files(&[(
        "example/reflect/reflect.xur",
        include_str!("../../../example/reflect/reflect.xur"),
    )]);
    assert_eq!(
        output,
        Ok(concat!(
            "robot: Robot\n  name = r2\n  pos: Pos\n    x = 0\n    y = 0\n  battery = 3\n",
            "commands: [\"forward\", \"right\", \"recharge\"]\n",
            "forward 2: now at (0, 2), battery 2\n",
            "right 1: now at (1, 2), battery 1\n",
            "jump 3: unknown command\n",
            "recharge: now at (1, 2), battery 100\n",
            "forward 5: now at (1, 7), battery 99\n",
        )
        .into())
    );
}

#[test]
fn layouts_follow_repr_and_field_attributes() {
    let src = r#"
        use std::reflect;
        #[repr(C)] struct C { a: u8, b: u64, c: u16 }
        #[repr(packed)] struct P { a: u8, b: u64, c: u16 }
        #[repr(align(32))] struct A { a: u8 }
        #[endian(Big)] struct B { a: u16, #[field(offset = 4)] b: i8 }
        struct N { inner: B, tail: u8 }
        fn main() {
            let io = std::io;
            io::println("{}", reflect::layout::<C>());
            io::println("{}", reflect::layout::<P>());
            io::println("{}", reflect::layout::<A>());
            let b = reflect::layout::<B>();
            let data = b.to_bytes(B { a: 0x0102, b: -2 });
            io::println("{} {}", data.to_hex(), b.from_bytes(data));
            io::println("{}", reflect::layout_of(N { inner: B { a: 1, b: 1 }, tail: 0 }));
        }
    "#;
    assert_eq!(
        run(src),
        Ok(concat!(
            "C (size 24, align 8) { a: u8 @ 0, b: u64 @ 8, c: u16 @ 16 }\n",
            "P (size 11, align 1) { a: u8 @ 0, b: u64 @ 1, c: u16 @ 9 }\n",
            "A (size 32, align 32) { a: u8 @ 0 }\n",
            "01020000fe00 B { a: 258, b: -2 }\n",
            "N (size 8, align 2) { inner: B @ 0, tail: u8 @ 6 }\n",
        )
        .into())
    );
}

#[test]
fn layout_errors() {
    let with =
        |items: &str, body: &str| run(&format!("use std::reflect; {items} fn main() {{ {body} }}"));
    assert_eq!(
        with(
            "struct S { a: u32, #[field(offset = 2)] b: u8 }",
            "reflect::layout::<S>();"
        ),
        Err("field `b` at offset 2 overlaps the field before it, which ends at 4".into())
    );
    assert_eq!(
        with("struct S { a: bytes }", "reflect::layout::<S>();"),
        Err("field `a`: `bytes` fields need a size: `#[field(size = n)]`".into())
    );
    assert_eq!(
        with("struct S { a: str }", "reflect::layout::<S>();"),
        Err("field `a`: `str` has no layout".into())
    );
    assert_eq!(
        with(
            "struct S { a: u8 }",
            "reflect::layout::<S>().to_bytes(S { a: 1 }); reflect::layout::<S>().write(b\"\", 0, S { a: 1 });"
        ),
        Err("`S` needs 1 byte(s) at offset 0, but there are only 0".into())
    );
    assert_eq!(
        with(
            "",
            "reflect::Layout::new(\"x\").field(\"a\", \"u8\").to_bytes((300,));"
        ),
        Err("no field `a` on tuple".into())
    );
    assert_eq!(
        with(
            "struct S { a: i64 }",
            "reflect::Layout::new(\"x\").field(\"a\", \"u8\").to_bytes(S { a: 300 });"
        ),
        Err("300 does not fit in field `a` (u8)".into())
    );
    assert_eq!(
        with("", "reflect::Layout::new(\"x\").field(\"a\", \"u3\");"),
        Err("`u3` is not a primitive type".into())
    );
}

#[test]
fn the_layout_example_runs() {
    let output = run_files(&[(
        "example/reflect/layout.xur",
        include_str!("../../../example/reflect/layout.xur"),
    )]);
    assert_eq!(output, Ok("Padded (size 8, align 4) { flag: u8 @ 0, value: u32 @ 4 }\nTagged (size 16, align 16) { tag: bytes[4] @ 0, value: u32 @ 8 }\n58555200000000000000cafe00000000\nBitmap (size 30, align 1) { file: FileHeader @ 0, info: InfoHeader @ 14 }\n38 bytes: 424d26000000000000001e000000100000000200000001000000010018000000ffff00000000\n2x1 pixels at offset 30\n  magic u16 at 0\n  size u32 at 2\n  reserved u32 at 6\n  pixels_at u32 at 10\nRecord (size 12, align 4) { id: u16 @ 0, score: f32 @ 4, alive: bool @ 8 }\n000000000000000000000000070000000000c03f01000000\n{ id: 7, score: 1.5, alive: true }\n".into()));
}

#[test]
fn reflection_names_variants() {
    let src = r#"
        enum Shape { Circle(f64), Square }
        struct Point { x: i64 }
        fn main() {
            let v = std::reflect::variant;
            std::io::println("{} {} {}", v(Shape::Circle(1.0)), v(Shape::Square), v(Point { x: 1 }));
        }
    "#;
    assert_eq!(
        run(src),
        Ok("Some(\"Circle\") Some(\"Square\") None\n".into())
    );
}

#[test]
fn structs_know_their_bytes() {
    let src = r#"
        use std::mem;
        #[endian(Big)]
        struct Header { magic: u16, #[field(offset = 4)] len: u32 }
        struct Own { a: u8 }
        impl Own { pub fn byte_size(self) -> str { "mine" } }
        enum Packet { Ping { id: u16 } }
        fn main() {
            let h = Header { magic: 0x4d5a, len: 3 };
            let io = std::io;
            io::println("{} {} {} {}", h.to_bytes().to_hex(), h.byte_size(), mem::size_of::<Header>(), mem::align_of::<Header>());
            io::println("{} {}", mem::from_bytes::<Header>(h.to_bytes()), mem::size_of_val(h));
            io::println("{} {}", Own { a: 1 }.byte_size(), Own { a: 1 }.to_bytes());
            io::println("{}", Packet::Ping { id: 1 }.to_bytes());
        }
    "#;
    assert_eq!(
        run(src),
        Ok(concat!(
            "4d5a000000000003 8 8 4\n",
            "Header { magic: 19802, len: 3 } 8\n",
            "mine b\"\\x01\"\n",
            "b\"\\x01\\x00\"\n",
        )
        .into())
    );
    assert_eq!(
        run("struct S { name: str } fn main() { S { name: \"x\" }.to_bytes(); }"),
        Err("field `name`: `str` has no layout".into())
    );
}

// Small, single-purpose `std::re` examples. The image ones build a PE in
// memory, so they are portable and deterministic; the proc ones read the
// current process, so they only run on the hosts the suite runs on.
fn re_image_example(file: &str, source: &str, expected: &str) {
    assert_eq!(run_files(&[(file, source)]), Ok(expected.into()));
}

#[test]
fn re_image_examples_run() {
    re_image_example(
        "example/re/image/build.xur",
        include_str!("../../../example/re/image/build.xur"),
        "built 1024 bytes, magic 5a4d\n",
    );
    re_image_example(
        "example/re/image/fields.xur",
        include_str!("../../../example/re/image/fields.xur"),
        "machine ImageFileMachine::Amd64\n",
    );
    re_image_example(
        "example/re/image/loaded.xur",
        include_str!("../../../example/re/image/loaded.xur"),
        "answer at 180002000: b82a000000c3\n",
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn re_proc_examples_run() {
    for (file, source) in [
        (
            "example/re/proc/modules.xur",
            include_str!("../../../example/re/proc/modules.xur"),
        ),
        (
            "example/re/proc/regions.xur",
            include_str!("../../../example/re/proc/regions.xur"),
        ),
        (
            "example/re/proc/read.xur",
            include_str!("../../../example/re/proc/read.xur"),
        ),
    ] {
        assert!(run_files(&[(file, source)]).is_ok(), "{file} failed");
    }
}

// pid and the loaded base address vary per run, so this checks the stable
// prefix rather than exact output.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn re_proc_inject_example_runs() {
    let output = run_files(&[(
        "example/re/proc/inject.xur",
        include_str!("../../../example/re/proc/inject.xur"),
    )])
    .unwrap();
    assert!(
        output.starts_with("loaded libc.so.6 into pid "),
        "{output:?}"
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn proc_inspects_the_current_process() {
    let src = r#"
        use std::re::proc;
        fn main() {
            let io = std::io;
            let handle = proc::open_current().unwrap();
            let modules = handle.modules().unwrap();
            io::println("{} {} {}", proc::current_pid() > 0, handle.pointer_width().unwrap(), modules.len() > 0);
            io::println("{} {}", handle.read(modules[0].base, 4).unwrap().len(), handle.is_alive());
            io::println("{}", proc::protection_names(7));
        }
    "#;
    assert_eq!(
        run(src),
        Ok("true 8 true\n4 true\n[\"READ\", \"WRITE\", \"EXECUTE\"]\n".into())
    );
}

#[test]
fn std_re_reports_bad_input() {
    assert_eq!(
        run("fn main() { std::io::println(\"{}\", std::re::AnyImage::load(b\"nope\").is_err()); }"),
        Ok("true\n".into())
    );
    assert_eq!(
        run("fn main() { std::re::pe::ImageDef::new_dll(\"Amd64\"); }"),
        Err("expected `ImageFileMachine`, found string".into())
    );
    assert_eq!(
        run("fn main() { std::io::println(\"{}\", std::re::pe::section_flags([\"NOPE\"])); }"),
        Ok("Err(\"no flag `NOPE`\")\n".into())
    );
}

#[test]
fn tuple_fields_are_read_and_assigned_by_number() {
    let src = r#"
        struct P { pair: (i64, (i64, i64)) }
        fn main() {
            let t = ((1, 2), (3, (4, 5)));
            let p = P { pair: (1, (2, 3)) };
            p.pair.1.0 = 20;
            t.0 = 9;
            std::io::println("{} {} {} {}", t.0, t.1.1.0, p, p.pair.1);
        }
    "#;
    assert_eq!(
        run(src),
        Ok("9 4 P { pair: (1, (20, 3)) } (20, 3)\n".into())
    );
    assert_eq!(
        run("fn main() { (1, 2).2; }"),
        Err("no field `2` on tuple".into())
    );
}

#[test]
fn rust_types_are_built_and_matched_like_xur_ones() {
    let src = r#"
        use std::re::{ExportLocation, pe::ImageFileMachine, pe::ImageDataDirectory};
        fn describe(m: ImageFileMachine) -> str {
            match m {
                ImageFileMachine::Amd64 => "x86-64",
                ImageFileMachine::Other(n) => "other " + n.to_hex(),
                _ => "something else",
            }
        }
        fn main() {
            let io = std::io;
            let dll = std::re::pe::ImageDef::new_dll(ImageFileMachine::Amd64);
            io::println("{} {}", dll.coff_header.machine == ImageFileMachine::Amd64, dll.coff_header.machine);
            io::println("{} {} {}", describe(ImageFileMachine::Amd64), describe(ImageFileMachine::Other(0x1234)), describe(ImageFileMachine::I386));
            let at = ExportLocation::Address(4096);
            match at {
                ExportLocation::Address(a) => io::println("{} {}", a, at.0),
                ExportLocation::Forwarder(name) => io::println("{}", name),
            }
            let dir = ImageDataDirectory { size: 16, virtual_address: 0x1000 };
            let ImageDataDirectory { virtual_address, .. } = dir;
            io::println("{} {} {}", dir, virtual_address, std::reflect::type_name(dir));
        }
    "#;
    assert_eq!(
        run(src),
        Ok(concat!(
            "true ImageFileMachine::Amd64\n",
            "x86-64 other 1234 something else\n",
            "4096 4096\n",
            "ImageDataDirectory { virtual_address: 4096, size: 16 } 4096 ImageDataDirectory\n",
        )
        .into())
    );
    let with = |body: &str| {
        run(&format!(
            "use std::re::{{ExportLocation, pe::ImageDataDirectory}}; fn main() {{ {body} }}"
        ))
    };
    assert_eq!(
        with("ExportLocation::Address(1, 2);"),
        Err("`ExportLocation::Address` takes 1 field(s), but 2 were given".into())
    );
    assert_eq!(
        with("ExportLocation::Address;"),
        Err("`ExportLocation::Address` takes 1 field(s), but 0 were given".into())
    );
    assert_eq!(
        with("ImageDataDirectory { size: 1 };"),
        Err("missing field `virtual_address` for `ImageDataDirectory`".into())
    );
    assert_eq!(
        with("ImageDataDirectory { size: 1, virtual_address: 2, extra: 3 };"),
        Err("`ImageDataDirectory` has no field `extra`".into())
    );
    assert_eq!(
        with("std::re::pe::ImageDef::new_dll(std::re::pe::ImageFileMachine::Other(-1));"),
        Err("-1 does not fit in `u16`".into())
    );
}

#[test]
fn any_image_gives_the_file_inside() {
    let src = r#"
        use std::re::{AnyImage, pe::{ImageDef, ImageFileMachine}};
        fn main() {
            let data = ImageDef::new_dll(ImageFileMachine::I386).write().unwrap();
            let image = AnyImage::load(data).unwrap();
            let pe = image.as_pe().unwrap();
            std::io::println("{} {}-bit {} {}", image.format(), if image.is_64() { 64 } else { 32 }, pe.coff_header.machine, image.as_elf());
        }
    "#;
    assert_eq!(
        run(src),
        Ok("pe 32-bit ImageFileMachine::I386 None\n".into())
    );
}

#[test]
fn main_may_return_a_result() {
    // `Ok` and plain values succeed with no output.
    assert_eq!(
        run("fn main() -> Result<i64, str> { Ok(7) }"),
        Ok(String::new())
    );
    assert_eq!(run("fn main() { 1 + 1; }"), Ok(String::new()));
    // `Err` from `main` fails the program, showing the error's value.
    let src = r#"
        fn bad() -> Result<i64, str> { Err("boom") }
        fn main() -> Result<(), str> {
            std::io::println("before");
            bad()?;
            std::io::println("after");
            Ok(())
        }
    "#;
    assert_eq!(run(src), Err("main returned an error: boom".into()));
}

#[test]
fn field_assignment_checks_the_declared_type() {
    let src = "struct P { x: u8, y: i64 } fn main() { let p = P { x: 1, y: 2 }; p.x = 5; p.y = p.x; std::io::println(\"{}\", p); }";
    assert_eq!(run(src), Ok("P { x: 5, y: 5 }\n".into()));
    let bad = "struct P { x: u8 } fn main() { let p = P { x: 1 }; p.x = 300; }";
    assert_eq!(run(bad), Err("expected `u8`, found 300".into()));
    // `reflect::set` goes through the same check.
    let reflect = "struct P { x: u8 } fn main() { std::reflect::set(P { x: 1 }, \"x\", 300); }";
    assert_eq!(run(reflect), Err("expected `u8`, found 300".into()));
    // An untyped or generic field is never checked.
    let loose = "struct P { x: _ } fn main() { let p = P { x: 1 }; p.x = \"now a string\"; std::io::println(\"{}\", p); }";
    assert_eq!(run(loose), Ok("P { x: \"now a string\" }\n".into()));
}

#[test]
fn struct_patterns_on_rust_types_must_name_every_field() {
    let whole = "use std::re::pe::ImageDataDirectory;
        fn main() {
            let d = ImageDataDirectory { virtual_address: 16, size: 8 };
            let io = std::io;
            match d { ImageDataDirectory { virtual_address, size } => io::println(\"{} {}\", virtual_address, size) }
            match d { ImageDataDirectory { virtual_address, .. } => io::println(\"{}\", virtual_address) }
            match d {
                ImageDataDirectory { virtual_address } => io::println(\"no\"),
                _ => io::println(\"needs all fields\"),
            }
        }";
    assert_eq!(run(whole), Ok("16 8\n16\nneeds all fields\n".into()));
    let dup = "use std::re::pe::ImageDataDirectory;
        fn main() { let d = ImageDataDirectory { virtual_address: 1, size: 2 }; match d { ImageDataDirectory { size: a, size: b } => 1 }; }";
    assert_eq!(run(dup), Err("field `size` bound more than once".into()));
}

#[test]
fn native_objects_without_display_show_their_type_name() {
    let src = "fn main() {
            let data = std::re::pe::ImageDef::new_dll(std::re::pe::ImageFileMachine::Amd64).write().unwrap();
            std::io::println(\"{}\", std::re::pe::PEFile::parse(data).unwrap());
        }";
    assert_eq!(run(src), Ok("<PEFile>\n".into()));
}

/// Runs `src`, returning the raw runtime error (with its notes), with
/// `?` tracing set to `tracing`.
fn run_for_error(src: &str, tracing: bool) -> crate::RuntimeError {
    let registry = Arc::new(xur_std::registry());
    let (program, diags) = Program::load(
        Path::new("main.xur"),
        Box::new(MemoryFileLoader::new([("main.xur", src)])),
        Box::new(Arc::clone(&registry)),
    )
    .unwrap();
    assert!(diags.is_empty(), "{diags:?}");
    let vm = Vm::new(program, registry, Box::new(Output::default()));
    vm.set_tracing(tracing);
    vm.run_main().unwrap_err()
}

#[test]
fn question_mark_traces_to_main_by_default() {
    let src = "
        fn inner() -> Result<i64, str> { Err(\"boom\") }
        fn middle() -> Result<i64, str> { Ok(inner()? + 1) }
        fn main() -> Result<(), str> { middle()?; Ok(()) }
    ";
    let error = run_for_error(src, true);
    assert_eq!(error.message, "main returned an error: boom");
    // Innermost `?` first: inner()? inside middle, then middle()? in main.
    assert_eq!(error.notes.len(), 2);
    assert!(
        error.notes.iter().all(|note| note.contains("`?`")),
        "{:?}",
        error.notes
    );
    // The runtime can turn tracing off.
    assert!(run_for_error(src, false).notes.is_empty());
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn std_process_spawns_and_waits_for_a_child() {
    let src = r#"
        fn main() -> Result<(), str> {
            let io = std::io;
            let child = std::process::spawn("true", [])?;
            io::println("pid > 0: {}", child.pid() > 0);
            io::println("exit code: {}", child.wait()?);
            Ok(())
        }
    "#;
    assert_eq!(run(src), Ok("pid > 0: true\nexit code: 0\n".into()));
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn std_process_spawn_reports_a_missing_program() {
    let src = r#"fn main() { std::io::println("{}", std::process::spawn("this-program-does-not-exist", []).is_err()); }"#;
    assert_eq!(run(src), Ok("true\n".into()));
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn std_process_kill_stops_a_running_child() {
    let src = r#"
        fn main() -> Result<(), str> {
            let child = std::process::spawn("sleep", ["5"])?;
            let still_running = child.try_wait()?;
            std::io::println("{}", still_running);
            child.kill()?;
            child.wait()?;
            Ok(())
        }
    "#;
    assert_eq!(run(src), Ok("None\n".into()));
}
