//! Private offline replay of a Debug capture; never prints bodies or error text.
#[cfg(all(unix, debug_assertions))]
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 2 {
        eprintln!("usage: stream_replay PRIVATE_CAPTURE");
        std::process::exit(2);
    }
    for chunk in [0, 1, 4096] {
        match hiroute_gateway::runtime::stream_capture::replay_capture(
            std::path::Path::new(&args[1]),
            chunk,
        )
        .await
        {
            Ok(summary) => println!("{summary}"),
            Err(reason) => {
                eprintln!("{reason}");
                std::process::exit(1);
            }
        }
    }
}
#[cfg(not(all(unix, debug_assertions)))]
fn main() {
    eprintln!("stream replay requires Unix Debug build");
    std::process::exit(2);
}
