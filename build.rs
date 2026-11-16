fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-dark".into()),
    )
    .expect("failed to compile Slint UI");
    println!("cargo:rerun-if-changed=ui");
}
