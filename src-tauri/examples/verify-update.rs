//! 只读发布物验证器：不构建 Tauri App、不联网、不执行安装器、不访问用户数据。
//! 与 tauri-plugin-updater 使用相同的清单解析器及 minisign-verify 验证库。
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use std::{error::Error, fs, path::Path};
use tauri_plugin_updater::RemoteRelease;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn decode(value: &str) -> Result<String> {
    Ok(String::from_utf8(STANDARD.decode(value.trim())?)?)
}

fn verify(config_path: &Path, artifacts: &Path) -> Result<()> {
    let config: serde_json::Value = serde_json::from_slice(&fs::read(config_path)?)?;
    let pubkey = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .ok_or("缺少公钥")?;
    let public_key = PublicKey::decode(&decode(pubkey)?)?;
    let release: RemoteRelease = serde_json::from_slice(&fs::read(artifacts.join("latest.json"))?)?;
    for target in ["windows-x86_64-nsis", "windows-x86_64-msi"] {
        let url = release.download_url(target)?;
        let name = url
            .path_segments()
            .and_then(|mut parts| parts.next_back())
            .ok_or("资产路径无效")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        {
            return Err("资产文件名不安全".into());
        }
        if fs::read_to_string(artifacts.join(format!("{name}.sig")))?.trim()
            != release.signature(target)?.trim()
        {
            return Err("独立签名文件与清单不一致".into());
        }
        let signature = Signature::decode(&decode(release.signature(target)?)?)?;
        public_key.verify(&fs::read(artifacts.join(name))?, &signature, true)?;
        // 只有经过 global signature 验证后才读取 trusted comment，防止重放旧包冒充新版本。
        let version = signature
            .trusted_comment()
            .split('\t')
            .find_map(|field| field.strip_prefix("version:"));
        let announced = release.version.to_string();
        if version != Some(announced.as_str()) {
            return Err("签名版本与清单版本不一致或缺少签名版本".into());
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let result = if args.len() == 3 {
        verify(Path::new(&args[1]), Path::new(&args[2]))
    } else {
        Err("用法：verify-update <tauri配置> <发布物目录>".into())
    };
    if let Err(error) = result {
        eprintln!("更新物验证失败：{error}");
        std::process::exit(1);
    }
    println!("Tauri 清单读取及两种安装器验签通过（未执行安装）");
}
