use std::path::Path;

use stellardb::namespace::Namespace;

pub fn backup(data: String, output: String) -> anyhow::Result<()> {
    let namespace = Namespace::open(Path::new(&data))?;
    namespace.backup_to(Path::new(&output))?;
    println!("Backup written to {}", output);
    Ok(())
}

pub fn restore(input: String, data: String) -> anyhow::Result<()> {
    Namespace::restore_from(Path::new(&input), Path::new(&data))?;
    println!("Backup restored to {}", data);
    Ok(())
}
