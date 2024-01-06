use std::io::Result;

fn main() -> Result<()> {
    prost_build::compile_protos(&["proto/bot_data.proto"], &[""])?;
    Ok(())
}
