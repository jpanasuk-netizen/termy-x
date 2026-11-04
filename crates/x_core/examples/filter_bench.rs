//! BEFORE/AFTER bench + equivalence oracle for local filtering hot paths.
//! Data is SYNTHETIC (deterministic, labelled): a 60-entry trend list in the
//! shapes X reports, plus a 420-post set (TIMELINE_INITIAL depth) whose
//! text/handle are screened through the same Trend-shaped haystacks.
//! Run: cargo run --release -p termy_x --example filter_bench
use std::time::Instant;
use termy_x::model::{Trend, TrendBucket, filter_trends, group_trends, sort_trends, trend_bucket};

// ---- verbatim pre-change implementations (the BEFORE / oracle) ----
fn bucket_old(category: Option<&str>) -> TrendBucket {
    let Some(raw) = category else {
        return TrendBucket::Other;
    };
    let c = raw.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| c.contains(n));
    if has(&["politic", "election", "government", "congress", "senate"]) {
        TrendBucket::Politics
    } else if has(&[
        "sport",
        "nfl",
        "nba",
        "mlb",
        "nhl",
        "soccer",
        "football",
        "baseball",
        "basketball",
        "tennis",
        "golf",
        "ufc",
        "wwe",
        "f1",
        "racing",
        "hockey",
        "olympic",
        "cricket",
        "boxing",
    ]) {
        TrendBucket::Sports
    } else if has(&["news", "breaking", "world news", "journalism"]) {
        TrendBucket::News
    } else if has(&[
        "entertainment",
        "music",
        "movie",
        "film",
        "tv",
        "television",
        "celebrity",
        "gaming",
        "game",
        "anime",
        "comedy",
        "streaming",
    ]) {
        TrendBucket::Entertainment
    } else if has(&[
        "tech",
        "science",
        "ai",
        "artificial intelligence",
        "software",
        "computing",
        "space",
        "crypto",
        "business",
        "finance",
    ]) {
        TrendBucket::Technology
    } else {
        TrendBucket::Other
    }
}
fn filter_old(trends: &[Trend], filter: &str) -> Vec<Trend> {
    let words: Vec<String> = filter
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    if words.is_empty() {
        return sort_trends(trends);
    }
    sort_trends(trends)
        .into_iter()
        .filter(|t| {
            let hay = format!(
                "{} {} {} {}",
                t.name.to_lowercase(),
                t.query.to_lowercase(),
                t.category.as_deref().unwrap_or_default().to_lowercase(),
                t.place.as_deref().unwrap_or_default().to_lowercase()
            );
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}
fn group_old(trends: &[Trend]) -> Vec<(TrendBucket, Vec<Trend>)> {
    let sorted = sort_trends(trends);
    TrendBucket::all()
        .into_iter()
        .filter_map(|bucket| {
            let items: Vec<Trend> = sorted
                .iter()
                .filter(|t| bucket_old(t.category.as_deref()) == bucket)
                .cloned()
                .collect();
            (!items.is_empty()).then_some((bucket, items))
        })
        .collect()
}

// ---- synthetic data (deterministic xorshift) ----
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}
const CATS: &[&str] = &[
    "Sports",
    "Politics",
    "News",
    "Technology",
    "Entertainment",
    "Trending in United States",
    "NFL · Trending",
    "Music",
    "Science",
    "",
    "Trending in Texas",
];
const WORDS: &[&str] = &[
    "rust", "termy", "nfl", "election", "breaking", "storm", "ai", "crypto", "space", "launch",
    "game", "final", "music", "live", "update", "market", "gol", "win", "season", "daily",
];
fn trend(name: String, cat: &str, rank: u64) -> Trend {
    Trend {
        query: name.clone(),
        name,
        volume: Some(rank * 100),
        place: Some("United States".into()),
        rank: Some(rank),
        category: Some(cat.into()),
    }
}
fn synth_trends(n: usize) -> Vec<Trend> {
    let mut r = Rng(0x9e3779b97f4a7c15);
    (0..n)
        .map(|i| {
            trend(
                format!("#{} {}", r.pick(WORDS), r.pick(WORDS)),
                r.pick(CATS),
                (i % 97 + 1) as u64,
            )
        })
        .collect()
}
fn synth_420_as_trends() -> Vec<Trend> {
    // 420 synthetic "tweets" screened via trend haystack shape
    let mut r = Rng(0x12345678abcdef);
    (0..420)
        .map(|i| {
            let text = format!(
                "{} {} {} {} {}",
                r.pick(WORDS),
                r.pick(WORDS),
                r.pick(WORDS),
                r.pick(WORDS),
                r.pick(WORDS)
            );
            Trend {
                name: text,
                query: format!("@user{}", i % 53),
                volume: None,
                place: Some("worldwide".into()),
                rank: Some((i + 1) as u64),
                category: Some(r.pick(CATS).into()),
            }
        })
        .collect()
}

fn timeit(label: &str, iters: usize, mut f: impl FnMut()) {
    f();
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    let us = t.elapsed().as_micros() as f64 / iters as f64;
    println!("{label:<58} {us:>10.2} us/call  ({iters} iters)");
}
fn main() {
    let trends60 = synth_trends(60);
    let tweets420 = synth_420_as_trends();
    let queries = [
        "",
        "nfl",
        "breaking storm",
        "ai rust",
        "breaking nfl xyz",
        "the live daily update",
    ];
    // equivalence first (byte-identical: full Trend equality, in order)
    for q in queries {
        assert_eq!(
            filter_old(&trends60, q),
            filter_trends(&trends60, q),
            "filter60 {q:?}"
        );
        assert_eq!(
            filter_old(&tweets420, q),
            filter_trends(&tweets420, q),
            "filter420 {q:?}"
        );
    }
    assert_eq!(group_old(&trends60), group_trends(&trends60));
    assert_eq!(group_old(&tweets420), group_trends(&tweets420));
    for t in trends60.iter().chain(&tweets420) {
        assert_eq!(
            bucket_old(t.category.as_deref()),
            trend_bucket(t.category.as_deref())
        );
    }
    println!("EQUIVALENCE OK (synthetic data): filter/group/bucket identical, order preserved");
    println!("--- BEFORE = verbatim old impl, AFTER = current pub impl (same process) ---");
    for q in ["nfl", "breaking storm", "ai rust launch"] {
        timeit(&format!("filter_trends 60  q={q:?} BEFORE"), 3000, || {
            std::hint::black_box(filter_old(&trends60, q));
        });
        timeit(&format!("filter_trends 60  q={q:?} AFTER"), 3000, || {
            std::hint::black_box(filter_trends(&trends60, q));
        });
        timeit(&format!("filter_trends 420 q={q:?} BEFORE"), 500, || {
            std::hint::black_box(filter_old(&tweets420, q));
        });
        timeit(&format!("filter_trends 420 q={q:?} AFTER"), 500, || {
            std::hint::black_box(filter_trends(&tweets420, q));
        });
    }
    timeit("group_trends 60 BEFORE", 3000, || {
        std::hint::black_box(group_old(&trends60));
    });
    timeit("group_trends 60 AFTER", 3000, || {
        std::hint::black_box(group_trends(&trends60));
    });
    timeit("group_trends 420 BEFORE", 500, || {
        std::hint::black_box(group_old(&tweets420));
    });
    timeit("group_trends 420 AFTER", 500, || {
        std::hint::black_box(group_trends(&tweets420));
    });

    let cats: Vec<Option<&str>> = tweets420.iter().map(|t| t.category.as_deref()).collect();
    timeit("trend_bucket x420 cats BEFORE", 3000, || {
        for c in &cats {
            std::hint::black_box(bucket_old(*c));
        }
    });
    timeit("trend_bucket x420 cats AFTER", 3000, || {
        for c in &cats {
            std::hint::black_box(trend_bucket(*c));
        }
    });
}
