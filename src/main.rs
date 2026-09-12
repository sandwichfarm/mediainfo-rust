fn main() {
    std::process::exit(mediainfo::cli::run(std::env::args().skip(1).collect()));
}
