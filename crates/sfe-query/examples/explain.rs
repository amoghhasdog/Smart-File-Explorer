//! `cargo run -p sfe-query --example explain -- videos longer than 10 minutes`
fn main() {
    let q = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    println!("{:#?}", sfe_query::parse_now(&q));
}
