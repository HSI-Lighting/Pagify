//! What a font file calls itself, by name record.
fn main() {
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        println!("== {path}");
        let Ok(face) = ttf_parser::Face::parse(&bytes, 0) else {
            println!("   not parseable");
            continue;
        };
        for name in face.names() {
            if let Some(text) = name.to_string() {
                println!(
                    "   id {:>3} platform {:?} — {text:?}",
                    name.name_id, name.platform_id
                );
            }
        }
        println!(
            "   bold: {}  italic: {}  weight: {:?}  variable: {} axis/axes",
            face.is_bold(),
            face.is_italic(),
            face.weight(),
            face.variation_axes().len()
        );
        // The outlines themselves, so a name table cannot flatter them.
        let upem = face.units_per_em();
        let stem = ["H", "o", "n"].iter().filter_map(|c| {
            let g = face.glyph_index(c.chars().next().unwrap())?;
            let advance = face.glyph_hor_advance(g)?;
            let bbox = face.glyph_bounding_box(g)?;
            Some(format!(
                "{c}: adv {} bbox {}x{}",
                advance * 1000 / upem,
                (bbox.x_max - bbox.x_min) as i32 * 1000 / upem as i32,
                (bbox.y_max - bbox.y_min) as i32 * 1000 / upem as i32
            ))
        }).collect::<Vec<_>>().join("  ");
        println!("   {stem}");
        println!("   glyphs: {}", face.number_of_glyphs());
    }
}
