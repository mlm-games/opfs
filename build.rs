fn main() {
    println!("cargo::rustc-check-cfg=cfg(web_sys_unstable_apis)");
    if std::env::var("CARGO_FEATURE_UNSTABLE_APIS").is_ok() {
        println!("cargo::rustc-cfg=web_sys_unstable_apis");
    }
}
