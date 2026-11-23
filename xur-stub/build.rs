fn main() {
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libxur_stub.so");
}
