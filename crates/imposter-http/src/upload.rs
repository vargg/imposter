//! Replayable upload bodies. Files are opened and read in 64 KiB chunks each time a body is
//! built, so a Digest retry or a 307/308 redirect sends them again without holding them in
//! memory. Contents are never kept anywhere else.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use futures_util::stream::{self, BoxStream, StreamExt as _};
use imposter_core::model::{FormItem, FormKind};
use tokio::io::AsyncReadExt as _;

pub const CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("Upload must reference a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("Cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Invalid multipart content type")]
    ContentType,
}

/// 48 hex digits, as the Python app uses.
pub fn boundary() -> String {
    let mut bytes = [0_u8; 24];
    if getrandom::fill(&mut bytes).is_err() {
        // A fixed boundary still works unless a part happens to contain it.
        bytes = [0x5a; 24];
    }
    bytes
        .iter()
        .fold(String::with_capacity(48), |mut out, byte| {
            // Writing to a String cannot fail.
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Check every referenced file before anything is sent: it must be a readable regular file.
pub async fn validate(paths: impl IntoIterator<Item = &Path>) -> Result<(), UploadError> {
    for path in paths {
        let read = |source| UploadError::Read {
            path: path.to_owned(),
            source,
        };
        let metadata = tokio::fs::metadata(path).await.map_err(read)?;
        if !metadata.is_file() {
            return Err(UploadError::NotAFile(path.to_owned()));
        }
        let mut file = tokio::fs::File::open(path).await.map_err(read)?;
        let mut probe = [0_u8; 1];
        file.read(&mut probe).await.map_err(read)?;
    }
    Ok(())
}

pub fn multipart_files(fields: &[FormItem]) -> impl Iterator<Item = &Path> {
    fields
        .iter()
        .filter(|f| f.kind == FormKind::File)
        .map(|f| Path::new(&f.value))
}

/// `"` and line breaks inside quoted header parameters are percent-encoded.
fn quote(value: &str) -> String {
    value
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace('"', "%22")
}

fn filename(field: &FormItem) -> String {
    if field.filename.is_empty() {
        Path::new(&field.value)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        field.filename.clone()
    }
}

/// The part header of one field, from `--boundary` to the blank line.
pub fn part_header(field: &FormItem, boundary: &str) -> Result<String, UploadError> {
    if field.content_type.contains(['\r', '\n']) {
        return Err(UploadError::ContentType);
    }
    let mut header = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"",
        quote(&field.name)
    );
    match field.kind {
        FormKind::File => {
            let filename = filename(field);
            let content_type = if field.content_type.is_empty() {
                mime_guess::from_path(&filename)
                    .first_raw()
                    .unwrap_or("application/octet-stream")
                    .to_owned()
            } else {
                field.content_type.clone()
            };
            let _ = write!(
                header,
                "; filename=\"{}\"\r\nContent-Type: {content_type}",
                quote(&filename)
            );
        }
        FormKind::Text if !field.content_type.is_empty() => {
            let _ = write!(header, "\r\nContent-Type: {}", field.content_type);
        }
        FormKind::Text => {}
    }
    header.push_str("\r\n\r\n");
    Ok(header)
}

pub type BodyStream = BoxStream<'static, Result<Bytes, UploadError>>;

/// A file as a stream of chunks, opened when first polled.
pub fn file_stream(path: PathBuf) -> BodyStream {
    stream::unfold(Some((path, None::<tokio::fs::File>)), |state| async move {
        let (path, file) = state?;
        let read = |source| UploadError::Read {
            path: path.clone(),
            source,
        };
        let mut file = match file {
            Some(file) => file,
            None => match tokio::fs::File::open(&path).await {
                Ok(file) => file,
                Err(error) => return Some((Err(read(error)), None)),
            },
        };
        let mut buffer = vec![0_u8; CHUNK_SIZE];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(count) => {
                buffer.truncate(count);
                Some((Ok(Bytes::from(buffer)), Some((path, Some(file)))))
            }
            Err(error) => Some((Err(read(error)), None)),
        }
    })
    .boxed()
}

/// A multipart body: each field's header, its text or file contents, then the closing boundary.
pub fn multipart_stream(fields: &[FormItem], boundary: &str) -> Result<BodyStream, UploadError> {
    let mut parts: Vec<BodyStream> = Vec::with_capacity(fields.len() * 3 + 1);
    for field in fields {
        parts.push(
            stream::once(std::future::ready(Ok(Bytes::from(part_header(
                field, boundary,
            )?))))
            .boxed(),
        );
        match field.kind {
            FormKind::File => parts.push(file_stream(PathBuf::from(&field.value))),
            FormKind::Text => {
                parts.push(
                    stream::once(std::future::ready(Ok(Bytes::from(field.value.clone())))).boxed(),
                );
            }
        }
        parts.push(stream::once(std::future::ready(Ok(Bytes::from_static(b"\r\n")))).boxed());
    }
    parts.push(
        stream::once(std::future::ready(Ok(Bytes::from(format!(
            "--{boundary}--\r\n"
        )))))
        .boxed(),
    );
    Ok(stream::iter(parts).flatten().boxed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_are_48_hex_digits_and_differ() {
        let a = boundary();
        assert_eq!(a.len(), 48);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, boundary());
    }

    #[test]
    fn content_types_with_line_breaks_are_rejected() {
        let field = FormItem {
            content_type: "text/plain\r\nX-Evil: 1".into(),
            ..FormItem::text("a", "b")
        };
        assert!(matches!(
            part_header(&field, "b"),
            Err(UploadError::ContentType)
        ));
    }

    #[tokio::test]
    async fn validation_rejects_missing_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert!(matches!(
            validate([missing.as_path()]).await,
            Err(UploadError::Read { .. })
        ));
        assert!(matches!(
            validate([dir.path()]).await,
            Err(UploadError::NotAFile(_))
        ));
        let file = dir.path().join("ok");
        std::fs::write(&file, b"x").unwrap();
        validate([file.as_path()]).await.unwrap();
    }

    #[tokio::test]
    async fn a_file_that_disappears_mid_body_is_a_stream_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut stream = file_stream(dir.path().join("gone"));
        assert!(matches!(
            stream.next().await,
            Some(Err(UploadError::Read { .. }))
        ));
        assert!(stream.next().await.is_none());
    }
}
