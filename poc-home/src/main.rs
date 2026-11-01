//! POC : récupérer la page Home de YouTube (browseId `FEwhat_to_watch`) via InnerTube.
//!
//! Usage :
//!   poc-home <cookies.txt>     # flux personnalisé, échoue si YouTube ne nous voit pas connecté
//!   poc-home --anon            # requête anonyme (pour vérifier la plomberie)
//!
//! Le fichier cookies.txt est au format Netscape (export "Get cookies.txt LOCALLY", yt-dlp, etc.).

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const ORIGIN: &str = "https://www.youtube.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const FALLBACK_CLIENT_VERSION: &str = "2.20261029.11.00";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let arg = args.next().context("usage: poc-home <cookies.txt> | --anon | --parse <browse.json>")?;
    if arg == "--parse" {
        let path = args.next().context("--parse <browse.json>")?;
        let data: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        let mut videos = Vec::new();
        collect_videos(&data, &mut videos);
        for v in &videos {
            println!("{:<12} {:>8}  {}  —  {}", v.id, v.duration, v.title, v.meta.join(" · "));
        }
        println!("\n{} vidéos", videos.len());
        return Ok(());
    }
    let cookies = if arg == "--anon" {
        HashMap::new()
    } else {
        load_netscape_cookies(&arg)?
    };

    let http = reqwest::blocking::Client::builder().user_agent(UA).build()?;
    let cookie_header = cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    let client_version = fetch_client_version(&http, &cookie_header).unwrap_or_else(|e| {
        eprintln!("warn: version client introuvable ({e}), fallback {FALLBACK_CLIENT_VERSION}");
        FALLBACK_CLIENT_VERSION.to_owned()
    });
    eprintln!("clientVersion = {client_version}");

    let body = json!({
        "context": { "client": {
            "clientName": "WEB",
            "clientVersion": client_version,
            "hl": "fr",
            "gl": "FR",
        }},
        "browseId": "FEwhat_to_watch",
    });

    let mut req = http
        .post(format!("{ORIGIN}/youtubei/v1/browse?prettyPrint=false"))
        .header("Origin", ORIGIN)
        .header("X-Origin", ORIGIN)
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", &client_version)
        .json(&body);
    if !cookies.is_empty() {
        req = req
            .header("Cookie", &cookie_header)
            .header("X-Goog-AuthUser", "0");
        if let Some(auth) = authorization_header(&cookies) {
            req = req.header("Authorization", auth);
        } else {
            bail!("ni SAPISID ni __Secure-3PAPISID dans le fichier de cookies");
        }
    }

    let resp = req.send()?;
    let status = resp.status();
    let data: Value = resp.json().context("réponse non JSON")?;
    if !status.is_success() {
        bail!("HTTP {status}: {}", data["error"]["message"]);
    }

    let logged_in = tracking_param(&data, "logged_in").unwrap_or_default();
    eprintln!("HTTP {status}, logged_in={logged_in}");

    let mut videos = Vec::new();
    collect_videos(&data, &mut videos);
    let shorts = count_key(&data, "shortsLockupViewModel");

    if videos.is_empty() {
        if let Some(nudge) = find_key(&data, "feedNudgeRenderer") {
            eprintln!("Home vide, message YouTube : {}", runs_text(&nudge["title"]));
        }
    }
    for v in &videos {
        println!("{:<12} {:>8}  {}  —  {}", v.id, v.duration, v.title, v.meta.join(" · "));
    }
    println!(
        "\n{} vidéos, {} shorts, continuation: {}",
        videos.len(),
        shorts,
        if find_key(&data, "continuationCommand").is_some() { "oui" } else { "non" }
    );

    if !cookies.is_empty() && logged_in != "1" {
        bail!(
            "YouTube ne nous considère pas connecté (logged_in={logged_in}). \
             Comparer avec la requête /browse copiée en cURL depuis les devtools."
        );
    }
    Ok(())
}

/// Lit un cookies.txt Netscape et garde les cookies des domaines youtube.com
/// (avec ou sans point initial).
fn load_netscape_cookies(path: &str) -> Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("lecture de {path}"))?;
    let mut out = HashMap::new();
    for line in text.lines() {
        // Les cookies HttpOnly sont préfixés par "#HttpOnly_" : ce ne sont pas des commentaires.
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let domain = f[0].trim_start_matches('.');
        if domain == "youtube.com" || domain.ends_with(".youtube.com") {
            out.insert(f[5].to_owned(), f[6].trim_end().to_owned());
        }
    }
    if out.is_empty() {
        bail!("aucun cookie youtube.com dans {path}");
    }
    Ok(out)
}

/// Reproduit l'en-tête envoyé par le client web :
/// `SAPISIDHASH <ts>_<sha1> SAPISID1PHASH <ts>_<sha1> SAPISID3PHASH <ts>_<sha1>`
/// avec sha1 = SHA1("<ts> <cookie> <origin>").
fn authorization_header(cookies: &HashMap<String, String>) -> Option<String> {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let hash = |value: &str| {
        let digest = Sha1::digest(format!("{ts} {value} {ORIGIN}").as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        format!("{ts}_{hex}")
    };
    let sapisid = cookies
        .get("SAPISID")
        .or_else(|| cookies.get("__Secure-3PAPISID"))?;
    let mut parts = vec![format!("SAPISIDHASH {}", hash(sapisid))];
    if let Some(v) = cookies.get("__Secure-1PAPISID") {
        parts.push(format!("SAPISID1PHASH {}", hash(v)));
    }
    if let Some(v) = cookies.get("__Secure-3PAPISID") {
        parts.push(format!("SAPISID3PHASH {}", hash(v)));
    }
    Some(parts.join(" "))
}

/// La version du client web change toutes les semaines : on la lit dans le ytcfg de la page.
fn fetch_client_version(http: &reqwest::blocking::Client, cookie_header: &str) -> Result<String> {
    let mut req = http.get(ORIGIN);
    if !cookie_header.is_empty() {
        req = req.header("Cookie", cookie_header);
    }
    let html = req.send()?.text()?;
    let key = "\"INNERTUBE_CLIENT_VERSION\":\"";
    let start = html.find(key).context("INNERTUBE_CLIENT_VERSION absent")? + key.len();
    let end = html[start..].find('"').context("ytcfg mal formé")?;
    Ok(html[start..start + end].to_owned())
}

struct Video {
    id: String,
    title: String,
    duration: String,
    meta: Vec<String>,
}

/// Home, abonnements et onglet Vidéos d'une chaîne partagent le même format :
/// richGridRenderer > richItemRenderer > content > lockupViewModel.
fn collect_videos(v: &Value, out: &mut Vec<Video>) {
    match v {
        Value::Object(map) => {
            if let Some(l) = map.get("lockupViewModel") {
                if l["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO" {
                    out.push(parse_lockup(l));
                    return;
                }
            }
            map.values().for_each(|x| collect_videos(x, out));
        }
        Value::Array(arr) => arr.iter().for_each(|x| collect_videos(x, out)),
        _ => {}
    }
}

fn parse_lockup(l: &Value) -> Video {
    let md = &l["metadata"]["lockupMetadataViewModel"];
    let meta = md["metadata"]["contentMetadataViewModel"]["metadataRows"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row["metadataParts"].as_array().into_iter().flatten())
        .filter_map(|p| p["text"]["content"].as_str().map(str::to_owned))
        .collect();
    let duration = l["contentImage"]["thumbnailViewModel"]["overlays"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|o| {
            o["thumbnailBottomOverlayViewModel"]["badges"][0]["thumbnailBadgeViewModel"]["text"]
                .as_str()
        })
        .unwrap_or("")
        .to_owned();
    Video {
        id: l["contentId"].as_str().unwrap_or("?").to_owned(),
        title: md["title"]["content"].as_str().unwrap_or("?").to_owned(),
        duration,
        meta,
    }
}

fn tracking_param(data: &Value, key: &str) -> Option<String> {
    data["responseContext"]["serviceTrackingParams"]
        .as_array()?
        .iter()
        .flat_map(|s| s["params"].as_array().into_iter().flatten())
        .find(|p| p["key"] == key)
        .and_then(|p| p["value"].as_str().map(str::to_owned))
}

fn find_key(v: &Value, key: &str) -> Option<Value> {
    match v {
        Value::Object(map) => map
            .get(key)
            .cloned()
            .or_else(|| map.values().find_map(|x| find_key(x, key))),
        Value::Array(arr) => arr.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

fn count_key(v: &Value, key: &str) -> usize {
    match v {
        Value::Object(map) => {
            map.contains_key(key) as usize + map.values().map(|x| count_key(x, key)).sum::<usize>()
        }
        Value::Array(arr) => arr.iter().map(|x| count_key(x, key)).sum(),
        _ => 0,
    }
}

fn runs_text(v: &Value) -> String {
    v["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["text"].as_str())
        .collect()
}
