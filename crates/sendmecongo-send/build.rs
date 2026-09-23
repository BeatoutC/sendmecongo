fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("app.ico");

        // winres 0.1 在 GNU 工具链上输出的是 `cargo:rustc-link-lib=static=resource`，
        // 而 GNU ld 不会把归档里的成员拉进来（没有任何符号引用它），图标就被静默丢掉了。
        // 这里改成把编译好的 resource.o 直接交给链接器，保证一定进最终的可执行文件。
        // 失败要显式报错，不能再让 `let _ =` 把它咽下去。
        if let Err(err) = res.compile() {
            panic!("winres 编译资源文件失败: {err}");
        }

        let obj = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("resource.o");
        if obj.exists() {
            println!("cargo:rustc-link-arg={}", obj.display());
        }
    }
}
