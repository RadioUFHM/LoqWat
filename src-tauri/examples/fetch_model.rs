//! Download and verify catalog models into the app's model directory.
//!   cargo run --release --example fetch_model -- base.en small.en
//!   cargo run --release --example fetch_model -- --list

use std::io::Write;
use std::sync::atomic::AtomicBool;

use loqwat_lib::models;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = models::models_dir()?;
    if args.is_empty() || args[0] == "--list" {
        println!("Model directory: {}", dir.display());
        for m in models::CATALOG {
            let path = m.path_in(&dir);
            let state = if path.exists() { "installed" } else { "-" };
            println!(
                "  {:<22} {:>5} MB  {:<10} {}",
                m.id,
                m.size / 1_000_000,
                state,
                m.label
            );
        }
        return Ok(());
    }
    let removed = models::clean_partial_downloads(&dir)?;
    if removed > 0 {
        println!("Removed {removed} partial download(s).");
    }
    let cancel = AtomicBool::new(false);
    for id in &args {
        let info = models::find(id).ok_or_else(|| anyhow::anyhow!("unknown model id {id}"))?;
        let path = info.path_in(&dir);
        if path.exists() {
            print!("{id}: present, verifying… ");
            std::io::stdout().flush()?;
            models::verify(info, &path)?;
            println!("OK");
            continue;
        }
        println!(
            "{id}: downloading {} ({} MB)",
            info.url(),
            info.size / 1_000_000
        );
        let mut last_pct = u64::MAX;
        let path = models::download(info, &dir, &cancel, |done, total| {
            let pct = done * 100 / total;
            if pct != last_pct && pct % 10 == 0 {
                print!(" {pct}%");
                let _ = std::io::stdout().flush();
                last_pct = pct;
            }
        })?;
        println!("\n{id}: verified → {}", path.display());
    }
    Ok(())
}
