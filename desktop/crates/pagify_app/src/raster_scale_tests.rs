use super::raster_scale;

/// The property that makes zooming smooth: a small change in zoom must
/// usually produce the *same* raster scale, so no re-render is asked for.
#[test]
fn nearby_zooms_share_a_raster_scale() {
    let mut distinct = std::collections::BTreeSet::new();
    // A pinch from 1x to 2x, at the granularity a trackpad delivers.
    for i in 0..=100 {
        let zoom = 1.0 + i as f32 / 100.0;
        distinct.insert(raster_scale(zoom).to_bits());
    }
    assert!(
        distinct.len() <= 5,
        "a 1x-2x pinch asked for {} different rasters; it used to ask for one per frame",
        distinct.len()
    );
}

/// And it must stay close enough that nobody sees the difference.
#[test]
fn the_raster_is_never_far_from_the_asked_for_size() {
    for i in 1..=400 {
        let asked = i as f32 / 20.0;
        let got = raster_scale(asked);
        let error = (got / asked).max(asked / got);
        assert!(
            error < 1.13,
            "at {asked:.2}x the page would be rasterised at {got:.2}x, {:.0}% out",
            (error - 1.0) * 100.0
        );
    }
}

#[test]
fn it_is_stable_and_monotonic() {
    let mut last = 0.0;
    for i in 1..=200 {
        let got = raster_scale(i as f32 / 10.0);
        assert!(got >= last, "raster scale went backwards as zoom increased");
        assert_eq!(got, raster_scale(i as f32 / 10.0), "not deterministic");
        last = got;
    }
}
