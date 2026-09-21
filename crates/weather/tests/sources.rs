use weather::sources::{cwa, hko, open_meteo};

#[test]
fn hko_line_always_says_hong_kong() {
    // B3: the requested name is ignored, in BOTH the line and the row.
    let body = r#"{"weatherForecast":[{"forecastWeather":"多雲","forecastMintemp":{"value":24},"forecastMaxtemp":{"value":30},"PSR":"高"}]}"#;
    let parsed = hko::parse(body).unwrap();
    let (line, row) = hko::format("九龍", &parsed);
    assert!(line.starts_with("🌤 香港："), "line was {line}");
    assert_eq!(row.as_ref().unwrap().location, "香港");
}

#[test]
fn hko_rain_has_no_percent_sign() {
    // B5: HKO uses 降雨概率{psr} with NO %, because PSR is qualitative.
    let body = r#"{"weatherForecast":[{"forecastWeather":"多雲","forecastMintemp":{"value":24},"forecastMaxtemp":{"value":30},"PSR":"高"}]}"#;
    let (line, _) = hko::format("香港", &hko::parse(body).unwrap());
    assert!(line.contains("降雨概率高"), "line was {line}");
    assert!(!line.contains("降雨概率高%"), "must not add a percent sign");
}

#[test]
fn hko_missing_temps_render_as_question_marks() {
    let body = r#"{"weatherForecast":[{"forecastWeather":"晴"}]}"#;
    let (line, row) = hko::format("香港", &hko::parse(body).unwrap());
    assert!(line.contains("低溫?°C"), "line was {line}");
    assert_eq!(row.unwrap().min_t, "?");
}

#[test]
fn hko_empty_forecast_warns_and_yields_no_row() {
    // B7: no row => does not count toward weather_data => can drive status to failed.
    let (line, row) = hko::format("香港", &hko::parse(r#"{"weatherForecast":[]}"#).unwrap());
    assert_eq!(line, "[WARN: HKO forecast unavailable for 香港]");
    assert!(row.is_none());
}

#[test]
fn cwa_null_location_key_is_an_empty_list_not_an_error() {
    // B9: `.get("location", []) or []` — present-but-null behaves as missing.
    assert_eq!(cwa::records(r#"{"records":{"location":null}}"#).unwrap().len(), 0);
    assert_eq!(cwa::records(r#"{"records":{}}"#).unwrap().len(), 0);
}

#[test]
fn open_meteo_rounds_half_to_even_like_python() {
    // B6. Verified: python round(24.5)==24 and round(26.5)==26, while Rust's
    // f64::round gives 25 and 27. A one-degree divergence on every .5 temp.
    assert_eq!(open_meteo::round_like_python(24.5), 24);
    assert_eq!(open_meteo::round_like_python(25.5), 26);
    assert_eq!(open_meteo::round_like_python(26.5), 26);
    assert_eq!(open_meteo::round_like_python(-24.5), -24);
    assert_eq!(open_meteo::round_like_python(24.4), 24);
    assert_eq!(open_meteo::round_like_python(24.6), 25);
}

#[test]
fn open_meteo_keeps_a_zero_rain_probability() {
    // B14: "0" is truthy in Python, so the field is rendered.
    let body = r#"{"daily":{"weather_code":[1],"temperature_2m_max":[30.0],"temperature_2m_min":[24.0],"precipitation_probability_max":[0]}}"#;
    let (line, _) = open_meteo::format("臺北市", &open_meteo::parse(body).unwrap());
    assert!(line.contains("降雨機率0%"), "line was {line}");
}

#[test]
fn open_meteo_success_line_is_suffixed_as_fallback() {
    let body = r#"{"daily":{"weather_code":[1],"temperature_2m_max":[30.0],"temperature_2m_min":[24.0],"precipitation_probability_max":[10]}}"#;
    let (line, row) = open_meteo::format("臺北市", &open_meteo::parse(body).unwrap());
    assert!(line.ends_with("（備援）"), "line was {line}");
    assert!(row.is_some());
}

#[test]
fn open_meteo_missing_arrays_warn_and_yield_no_row() {
    let (line, row) = open_meteo::format("臺北市", &open_meteo::parse(r#"{"daily":{}}"#).unwrap());
    assert_eq!(line, "[WARN: Open-Meteo forecast unavailable for 臺北市]");
    assert!(row.is_none());
}

#[test]
fn cwa_slot_selection_is_stable_for_a_past_only_fixture() {
    // B15: the picker reads the wall clock. Fixtures must be built so the
    // choice is time-invariant; this asserts the fixture shape the differential
    // harness depends on. If this test starts flaking, the fixture is wrong,
    // not the code.
    let body = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/differential/fixtures/cwa_past_only.json")
    ).unwrap();
    let recs = cwa::records(&body).unwrap();
    let (line_a, _) = cwa::format("臺北市", &recs[0]);
    let (line_b, _) = cwa::format("臺北市", &recs[0]);
    assert_eq!(line_a, line_b);
    assert!(line_a.contains("臺北市"), "line was {line_a}");
}
