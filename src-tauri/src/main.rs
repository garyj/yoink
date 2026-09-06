fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--help"] | ["-h"] => println!("yoink - clipboard history for Linux X11\n\nUsage: yoink [--quit | --version | --help]\n\nStarts hidden. Run again to show or hide the window.\n--quit stops the running instance without changing history."),
        ["--version"] | ["-V"] => println!("yoink {}", env!("CARGO_PKG_VERSION")),
        [] => yoink_lib::run(false),
        ["--quit"] => yoink_lib::run(true),
        _ => {
            eprintln!("yoink: unknown arguments; run yoink --help");
            std::process::exit(2);
        }
    }
}
