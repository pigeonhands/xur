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
            "example/main.xur",
            include_str!("../../../example/main.xur"),
        ),
        (
            "example/math.xur",
            include_str!("../../../example/math.xur"),
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
        "example/channels.xur",
        include_str!("../../../example/channels.xur"),
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
    let src = include_str!("../../../example/echo_server.xur");
    let loader = MemoryFileLoader::new([("example/echo_server.xur", src)]);
    let (_, diags) = Program::load(
        Path::new("example/echo_server.xur"),
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
        "example/bytes.xur",
        include_str!("../../../example/bytes.xur"),
    )]);
    assert_eq!(
        output,
        Ok("00000000  58 55 52 21 01 00 03 00 ad de 00 00 ef be 00 00\n00000010  2a 00 00 00\nentries: Ok([57005, 48879, 1337])\nErr(\"not a XUR! file\")\nmagic at 1\n".into())
    );
}
