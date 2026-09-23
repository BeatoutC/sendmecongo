fn main() {
    // 判断的是**被编译的目标**是不是 Windows，而不是宿主。
    // build script 永远由宿主编译并运行，所以 `#[cfg(target_os = "windows")]` 在这里
    // 反映的是"编译这个脚本的机器"——在 macOS / Linux 上交叉编译到 Windows 时它整个
    // 不成立，winres 根本不会跑，图标就静默丢了。CARGO_CFG_TARGET_OS 才是目标平台，
    // 这也和 Cargo.toml 里那条 build-dependency 的 cfg 语义对上了。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winres::WindowsResource::new();
    res.set_icon("app.ico");

    // winres 0.1 在 GNU 工具链上输出的是 `cargo:rustc-link-lib=static=resource`，
    // 而 GNU ld 不会把归档里的成员拉进来（没有任何符号引用它），图标就被静默丢掉了。
    // 这里改成把编译好的 resource.o 直接交给链接器，保证一定进最终的可执行文件。
    //
    // 例外：交叉编译时宿主上通常没有资源编译器（windres / rc.exe）。那是环境问题、
    // 不是代码问题，但也不能装作成功——明确警告一次，并且不追加 link-arg。
    let crossing = std::env::var("HOST").ok() != std::env::var("TARGET").ok();
    if let Err(err) = res.compile() {
        if crossing {
            println!(
                "cargo:warning=交叉编译到 Windows，但宿主缺少资源编译器（windres / rc.exe），\
                 本次产物不会带图标: {err}"
            );
            return;
        }
        panic!("winres 编译资源文件失败: {err}");
    }

    let obj = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("resource.o");
    if obj.exists() {
        println!("cargo:rustc-link-arg={}", obj.display());
    }
}
