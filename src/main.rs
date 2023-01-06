use clap::Parser;
mod p0_tcp;
mod p1_prime;
mod p2_means;
mod p3_chat;
mod p4_db;
mod p5_middle;
mod prelude;
mod utils;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Problem Id from https://protohackers.com/
    problem_id: u64,
}

fn main() {
    let args = Args::parse();
    let executor = match args.problem_id {
        0 => p0_tcp::main,
        1 => p1_prime::main,
        2 => p2_means::main,
        3 => p3_chat::main,
        4 => p4_db::main,
        5 => p5_middle::main,
        _ => unimplemented!(),
    };
    executor().unwrap();
}
