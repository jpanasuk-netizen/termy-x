//! Lane-1 perf harness: parse + local-filter timing on synthetic
//! provider payloads. Run: cargo run --release --example perf_bench
//! Prints median ms per stage so allocator/parser changes can be
//! compared before/after. No network, no posting.
use std::time::Instant;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn synthetic_posts(n: usize) -> String {
    let mut rows = String::from("[");
    for i in 0..n {
        if i > 0 {
            rows.push(',');
        }
        rows.push_str(&format!(
            r#"{{"id":"{}","text":"post {} about rust and termy-x shipping fast, with some longer body text to chew on #rust @someone https://example.com/x","user":{{"username":"user{}","name":"User {}"}},"like_count":{},"retweet_count":{},"reply_count":3,"bookmark_count":1,"views":{},"media_urls":["https://pbs.twimg.com/media/ABC{}?format=jpg&name=900x900"]}}"#,
            2_100_000_000_000_000_000u64 + i as u64, i, i % 97, i % 97, i * 7, i * 3, i * 100, i
        ));
    }
    rows.push(']');
    rows
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let payload = synthetic_posts(420);
    println!("payload_bytes={}", payload.len());
    // Parse stage (provider JSON -> Vec<Post>)
    let mut times = Vec::new();
    for _ in 0..15 {
        let t = Instant::now();
        let posts = termy_x::providers::parse_posts(&payload).expect("parse");
        assert_eq!(posts.len(), 420);
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("parse_posts_420 median_ms={:.3}", median(times));
    // Filter stage (local trends filter hot path)
    let trends: Vec<termy_x::model::Trend> = (0..500)
        .map(|i| termy_x::model::Trend {
            name: format!("Trend {i} rust news"),
            query: format!("rust {i}"),
            category: Some("Tech".into()),
            place: Some("United States".into()),
            rank: Some(i as u64 + 1),
            volume: None,
        })
        .collect();
    let mut times = Vec::new();
    for _ in 0..15 {
        let t = Instant::now();
        let out = termy_x::model::filter_trends(&trends, "rust tech");
        assert!(!out.is_empty());
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("filter_trends_500 median_ms={:.3}", median(times));
    // Alloc churn stage (post clone/dedupe-like work)
    let posts = termy_x::providers::parse_posts(&payload).expect("parse");
    let mut times = Vec::new();
    for _ in 0..15 {
        let t = Instant::now();
        let mut acc = 0usize;
        for _ in 0..20 {
            let c = posts.clone();
            acc += c.len();
        }
        assert_eq!(acc, 420 * 20);
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!("clone_posts_x20 median_ms={:.3}", median(times));
}
