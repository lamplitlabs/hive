fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(hive::run_streaming(
        &args,
        Box::new(std::io::stdout()),
        Box::new(std::io::stderr()),
        hive::Terminal::of_process(),
    ));
}
