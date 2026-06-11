fn main() {
    println!("cargo:rerun-if-changed=../newflasher.c");
    println!("cargo:rerun-if-changed=../newflasher.h");

    cc::Build::new()
        .file("../newflasher.c")
        .file("../newflasher.h")
        .define("main", "main_c")
        .warnings(false)
        .compile("newflasher_c");
}