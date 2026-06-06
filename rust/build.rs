fn main() {
    println!("cargo:rerun-if-changed=../newflasher.c");

    cc::Build::new()
        .file("../newflasher.c")
        .define("main", "main_c")
        .warnings(false)
        .compile("newflasher_c");
}