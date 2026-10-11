//! Mise au point : aperçu PNG d'un projet .graf, comme l'accueil le calcule pour un modèle.
//!
//!     cargo run -p engine --release --example project_preview -- modele.graf apercu.png 640
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (input, output) = (args.next().ok_or("projet")?, args.next().ok_or("sortie")?);
    let size: u32 = args.next().map_or(Ok(640), |s| s.parse())?;
    let started = std::time::Instant::now();
    let doc = engine::io::load_project(std::path::Path::new(&input))?;
    let loaded = started.elapsed();
    let png = engine::io::preview_png(&doc, size)?;
    std::fs::write(&output, &png)?;
    println!("{} × {} · {} calques · chargé en {loaded:?}, aperçu en {:?} → {output}", doc.width(), doc.height(), doc.walk_top_down().len(), started.elapsed() - loaded);
    Ok(())
}
