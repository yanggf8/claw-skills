fn main() {
    let titles: Vec<String> =
        serde_json::from_str(&std::fs::read_to_string("/tmp/titles.json").unwrap()).unwrap();
    let mut out = serde_json::Map::new();
    for t in titles {
        let mut w: Vec<String> = news::text::topic_words(&t).into_iter().collect();
        w.sort();
        out.insert(t, serde_json::json!(w));
    }
    println!("{}", serde_json::to_string(&out).unwrap());
}
