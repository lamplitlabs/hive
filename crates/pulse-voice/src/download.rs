//! Fetching the runtime and a model: streamed to a `.part` file next to the
//! target, resumed with a byte range when a part is already there, hashed
//! while it is written, renamed into place only when the hash matches and
//! marked with a `.sha256` sidecar so a later status read trusts it without
//! hashing 2.5 GB again.
//!
//! Nothing here holds the whole file in memory. Progress goes to a callback
//! so the console's dialog and `pulse voice setup` can both show it.

use crate::catalogue::RuntimeSpec;
use crate::paths::{partial, sidecar};
use anyhow::{Context as _, Result, anyhow, bail};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

/// How far one download got, for progress reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// One read of the socket: 1 MiB keeps the syscall count low on a 2.5 GB
/// file and the progress callback at a few hundred calls.
const CHUNK: usize = 1 << 20;

/// How long one read may stall before the download is given up (the
/// request as a whole has no cap: 2.5 GB over a slow line takes as long as
/// it takes).
pub const READ_STALL: Duration = Duration::from_secs(120);

/// A client for the voice downloads: a connect cap, a read stall cap, and
/// pulse's own user agent so the mirrors see who is asking.
pub fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("pulse-voice/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(READ_STALL)
        .build()
        .context("building the download client")
}

/// Whether a download's error means the caller should not retry the same
/// URL (the server said the file is not there or refused).
pub fn permanent(status: reqwest::StatusCode) -> bool {
    status.is_client_error() && status != reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// Downloads `url` to `target`: resumes `target.part` when it exists, checks
/// `size` and `sha256`, writes the sidecar. `progress` is called with the
/// bytes on disk so far (including the resumed ones) and the total; return
/// `false` from it to stop (the part stays for a later resume).
pub fn fetch(
    client: &reqwest::blocking::Client,
    url: &str,
    target: &Path,
    size: u64,
    sha256: &str,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent folder", target.display()))?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let part = partial(target);

    // What is already there, hashed from the start so the digest covers the
    // whole file whichever request wrote which half of it.
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut f) = File::open(&part) {
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            have += n as u64;
        }
    }
    if have > size {
        // A part longer than the file is from another release of it.
        let _ = fs::remove_file(&part);
        have = 0;
        hasher = Sha256::new();
    }
    if !progress(Progress {
        done: have,
        total: size,
    }) {
        bail!("stopped");
    }

    if have < size {
        let mut req = client.get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let mut resp = req.send().with_context(|| format!("fetching {url}"))?;
        let status = resp.status();
        let mut file = if have > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT {
            OpenOptions::new().append(true).open(&part)?
        } else if status.is_success() {
            // The server ignored the range (or there was none): start over.
            have = 0;
            hasher = Sha256::new();
            File::create(&part)?
        } else {
            bail!("{url} answered HTTP {status}");
        };
        if let Some(len) = resp.content_length()
            && have + len != size
        {
            bail!(
                "{url} is {} bytes, not the {size} this release of pulse expects; the file changed upstream",
                have + len
            );
        }
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = resp
                .read(&mut buf)
                .with_context(|| format!("reading {url} at byte {have}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            have += n as u64;
            if have > size {
                bail!("{url} sent more than the {size} bytes this release of pulse expects");
            }
            if !progress(Progress {
                done: have,
                total: size,
            }) {
                file.flush()?;
                bail!("stopped");
            }
        }
        file.flush()?;
        drop(file);
        if have < size {
            bail!("{url} ended after {have} of {size} bytes; run the setup again to resume");
        }
    }

    let got = hex::encode(hasher.finalize());
    if got != sha256 {
        let _ = fs::remove_file(&part);
        bail!(
            "{} does not match its published hash (got {got}, want {sha256}); the download was removed, run the setup again",
            target.display()
        );
    }
    fs::rename(&part, target).with_context(|| format!("moving {} into place", target.display()))?;
    fs::write(sidecar(target), format!("{sha256}\n"))?;
    Ok(())
}

/// Takes the one library out of the runtime's `.tgz` at `archive` and
/// writes it to `target`, checking its size and hash. The archive is read
/// once in a stream: gzip, then the tar headers, skipping every other
/// member. The release archives are plain ustar/GNU with no long-name
/// extension on the member we want (its path is under 100 characters), so
/// the header's name and prefix fields are enough.
pub fn extract_library(archive: &Path, runtime: &RuntimeSpec, target: &Path) -> Result<()> {
    let file = File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut tar = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut header = [0u8; 512];
    let mut found = false;
    loop {
        if read_full(&mut tar, &mut header)? == 0 || header.iter().all(|b| *b == 0) {
            break;
        }
        let name = tar_name(&header);
        let size = tar_size(&header)?;
        let kind = header[156];
        if name == runtime.member && (kind == b'0' || kind == 0) {
            if size != runtime.library_size {
                bail!(
                    "{} holds {size} bytes of {}, not the {} this release expects",
                    archive.display(),
                    runtime.member,
                    runtime.library_size
                );
            }
            if let Some(dir) = target.parent() {
                fs::create_dir_all(dir)?;
            }
            let part = partial(target);
            let mut out = File::create(&part)?;
            let mut hasher = Sha256::new();
            let mut left = size;
            let mut buf = vec![0u8; CHUNK];
            while left > 0 {
                let want = (left as usize).min(buf.len());
                let n = tar.read(&mut buf[..want])?;
                if n == 0 {
                    bail!("{} ends inside {}", archive.display(), runtime.member);
                }
                out.write_all(&buf[..n])?;
                hasher.update(&buf[..n]);
                left -= n as u64;
            }
            out.flush()?;
            drop(out);
            let got = hex::encode(hasher.finalize());
            if got != runtime.library_sha256 {
                let _ = fs::remove_file(&part);
                bail!(
                    "{} in {} does not match its published hash (got {got}, want {})",
                    runtime.member,
                    archive.display(),
                    runtime.library_sha256
                );
            }
            fs::rename(&part, target)?;
            fs::write(sidecar(target), format!("{}\n", runtime.library_sha256))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(target, fs::Permissions::from_mode(0o755))?;
            }
            found = true;
            break;
        }
        skip(&mut tar, size.div_ceil(512) * 512)?;
    }
    if !found {
        bail!(
            "{} holds no {}; the archive is not the ONNX Runtime {} release",
            archive.display(),
            runtime.member,
            runtime.version
        );
    }
    Ok(())
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        let n = r.read(&mut buf[got..])?;
        if n == 0 {
            break;
        }
        got += n;
    }
    Ok(got)
}

fn skip(r: &mut impl Read, mut n: u64) -> Result<()> {
    let mut buf = vec![0u8; CHUNK];
    while n > 0 {
        let want = (n as usize).min(buf.len());
        let got = r.read(&mut buf[..want])?;
        if got == 0 {
            bail!("the archive ends early");
        }
        n -= got as u64;
    }
    Ok(())
}

fn tar_field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn tar_name(header: &[u8; 512]) -> String {
    let name = tar_field(&header[0..100]);
    // Only POSIX ustar keeps a path prefix at 345; GNU tar ("ustar ") uses
    // those bytes for other things and writes long names as a separate
    // member, which the release archives do not need for our file.
    let prefix = if &header[257..263] == b"ustar\0" {
        tar_field(&header[345..500])
    } else {
        String::new()
    };
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

fn tar_size(header: &[u8; 512]) -> Result<u64> {
    let field = &header[124..136];
    if field[0] & 0x80 != 0 {
        // GNU base-256 size for files over 8 GB: not in these archives.
        let mut n: u64 = 0;
        for b in &field[1..] {
            n = (n << 8) | *b as u64;
        }
        return Ok(n);
    }
    let text = tar_field(field);
    let text = text.trim_matches(|c| c == ' ' || c == '\0');
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8).map_err(|_| anyhow!("a tar header has a bad size field"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::verified;
    use std::net::TcpListener;

    fn sha(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    /// A one-request HTTP server that honours a Range header; returns the
    /// URL and the thread to join.
    fn serve(body: Vec<u8>, honour_range: bool) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let range = req
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("range").then(|| v.trim())
                })
                .and_then(|r| r.strip_prefix("bytes="))
                .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
            let (status, slice) = match range {
                Some(from) if honour_range => ("206 Partial Content", &body[from..]),
                _ => ("200 OK", &body[..]),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                slice.len()
            );
            sock.write_all(head.as_bytes()).unwrap();
            sock.write_all(slice).unwrap();
            req
        });
        (url, handle)
    }

    #[test]
    fn fetch_resumes_a_part_and_writes_the_sidecar() {
        let body: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let hash = sha(&body);
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("file.bin");
        fs::write(partial(&target), &body[..1200]).unwrap();
        let (url, server) = serve(body.clone(), true);
        let mut seen = Vec::new();
        fetch(
            &client().unwrap(),
            &url,
            &target,
            body.len() as u64,
            &hash,
            &mut |p| {
                seen.push(p.done);
                true
            },
        )
        .unwrap();
        let req = server.join().unwrap();
        assert!(
            req.to_ascii_lowercase().contains("range: bytes=1200-"),
            "{req}"
        );
        assert!(req.contains("pulse-voice/"), "{req}");
        assert_eq!(fs::read(&target).unwrap(), body);
        assert!(verified(&target, 5000, &hash));
        assert!(!partial(&target).exists());
        assert_eq!(seen[0], 1200, "the first report counts the part");
        assert_eq!(*seen.last().unwrap(), 5000);
    }

    #[test]
    fn fetch_starts_over_when_the_server_ignores_the_range() {
        let body = b"0123456789".to_vec();
        let hash = sha(&body);
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("file.bin");
        fs::write(partial(&target), b"xxx").unwrap();
        let (url, server) = serve(body.clone(), false);
        fetch(&client().unwrap(), &url, &target, 10, &hash, &mut |_| true).unwrap();
        server.join().unwrap();
        assert_eq!(fs::read(&target).unwrap(), body);
    }

    #[test]
    fn fetch_refuses_a_bad_hash_or_size_and_removes_the_part() {
        let body = b"hello world".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("file.bin");
        let (url, server) = serve(body.clone(), true);
        let err = fetch(
            &client().unwrap(),
            &url,
            &target,
            11,
            &sha(b"other"),
            &mut |_| true,
        )
        .unwrap_err()
        .to_string();
        server.join().unwrap();
        assert!(err.contains("does not match its published hash"), "{err}");
        assert!(!target.exists() && !partial(&target).exists());

        let (url, server) = serve(body.clone(), true);
        let err = fetch(
            &client().unwrap(),
            &url,
            &target,
            99,
            &sha(&body),
            &mut |_| true,
        )
        .unwrap_err()
        .to_string();
        server.join().unwrap();
        assert!(
            err.contains("not the 99 this release of pulse expects"),
            "{err}"
        );
    }

    #[test]
    fn fetch_stops_when_the_callback_says_so_and_keeps_the_part() {
        let body: Vec<u8> = vec![7u8; 3000];
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("file.bin");
        let (url, server) = serve(body.clone(), true);
        let mut calls = 0;
        let err = fetch(
            &client().unwrap(),
            &url,
            &target,
            3000,
            &sha(&body),
            &mut |p| {
                calls += 1;
                p.done == 0
            },
        )
        .unwrap_err()
        .to_string();
        let _ = server.join();
        assert_eq!(err, "stopped");
        assert!(partial(&target).exists());
        assert!(!target.exists());
        assert!(calls >= 2);
    }

    fn tgz(members: &[(&str, &[u8], u8)]) -> Vec<u8> {
        let mut raw = Vec::new();
        for (name, data, kind) in members {
            let mut h = [0u8; 512];
            // Split a long path into ustar prefix/name like the release tars do.
            let (prefix, base) = if name.len() > 100 {
                let cut = name[..100].rfind('/').unwrap();
                (&name[..cut], &name[cut + 1..])
            } else {
                ("", *name)
            };
            h[..base.len()].copy_from_slice(base.as_bytes());
            h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
            h[100..108].copy_from_slice(b"0000755\0");
            h[108..116].copy_from_slice(b"0000000\0");
            h[116..124].copy_from_slice(b"0000000\0");
            let size = format!("{:011o}\0", data.len());
            h[124..136].copy_from_slice(size.as_bytes());
            h[136..148].copy_from_slice(b"00000000000\0");
            h[156] = *kind;
            h[257..263].copy_from_slice(b"ustar\0");
            h[263..265].copy_from_slice(b"00");
            h[148..156].copy_from_slice(b"        ");
            let sum: u32 = h.iter().map(|b| *b as u32).sum();
            let chk = format!("{sum:06o}\0 ");
            h[148..156].copy_from_slice(chk.as_bytes());
            raw.extend_from_slice(&h);
            raw.extend_from_slice(data);
            let pad = (512 - data.len() % 512) % 512;
            raw.extend(std::iter::repeat_n(0u8, pad));
        }
        raw.extend(std::iter::repeat_n(0u8, 1024));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn extract_library_finds_the_member_behind_other_entries() {
        let lib = b"not really a dylib but long enough to matter".to_vec();
        let lib_hash = sha(&lib);
        let spec = RuntimeSpec {
            version: "9.9.9",
            platform: "test",
            archive_size: 0,
            archive_sha256: "",
            member: "onnxruntime-test-9.9.9/lib/libonnxruntime.9.9.9.dylib",
            library: "libonnxruntime.9.9.9.dylib",
            library_size: lib.len() as u64,
            library_sha256: Box::leak(lib_hash.clone().into_boxed_str()),
        };
        let long = "onnxruntime-test-9.9.9/lib/libonnxruntime.9.9.9.dylib.dSYM/Contents/Resources/DWARF/deep/deeper/libonnxruntime.9.9.9.dylib";
        let archive = tgz(&[
            ("onnxruntime-test-9.9.9/", b"", b'5'),
            ("onnxruntime-test-9.9.9/LICENSE", b"MIT", b'0'),
            (long, b"debug symbols", b'0'),
            (spec.member, &lib, b'0'),
            ("onnxruntime-test-9.9.9/VERSION", b"9.9.9", b'0'),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let tgz_path = dir.path().join("rt.tgz");
        fs::write(&tgz_path, &archive).unwrap();
        let target = dir.path().join("runtime").join(spec.library);
        extract_library(&tgz_path, &spec, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), lib);
        assert!(verified(&target, lib.len() as u64, &lib_hash));

        let mut wrong = spec;
        wrong.member = "onnxruntime-test-9.9.9/lib/missing.dylib";
        let err = extract_library(&tgz_path, &wrong, &target)
            .unwrap_err()
            .to_string();
        assert!(err.contains("holds no"), "{err}");

        let mut bad = spec;
        bad.library_sha256 = "00";
        let err = extract_library(&tgz_path, &bad, &target)
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not match its published hash"), "{err}");
    }

    #[test]
    fn tar_headers_parse_names_prefixes_and_sizes() {
        let mut h = [0u8; 512];
        h[..3].copy_from_slice(b"lib");
        h[257..263].copy_from_slice(b"ustar\0");
        h[345..351].copy_from_slice(b"prefix");
        h[124..136].copy_from_slice(b"00000000012\0");
        assert_eq!(tar_name(&h), "prefix/lib");
        assert_eq!(tar_size(&h).unwrap(), 10);
        h[124..136].copy_from_slice(b"           \0");
        assert_eq!(tar_size(&h).unwrap(), 0);
        h[124..136].copy_from_slice(&[0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5]);
        assert_eq!(tar_size(&h).unwrap(), 5);
        let mut gnu = [0u8; 512];
        gnu[..4].copy_from_slice(b"name");
        gnu[257..263].copy_from_slice(b"ustar ");
        gnu[345..348].copy_from_slice(b"xxx");
        assert_eq!(tar_name(&gnu), "name", "GNU tar keeps no prefix at 345");
    }
}
