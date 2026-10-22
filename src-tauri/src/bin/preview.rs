use std::io::Read;

use folio_core::{
    document::{Edit, Error, MAX_DOCUMENT_BYTES},
    workspace::Workspace,
};
use serde::Deserialize;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Revision {
    document_id: u64,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditRequest {
    document_id: u64,
    revision: u64,
    edit: Edit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateRequest {
    width: f32,
    height: f32,
    discard_changes: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtractRequest {
    document_id: u64,
    revision: u64,
    pages: Vec<usize>,
}

enum Payload {
    Json(serde_json::Value),
    Pdf(Vec<u8>),
}

fn header(request: &Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str().to_owned())
}

fn body(request: &mut Request) -> Result<Vec<u8>, Error> {
    if request
        .body_length()
        .is_some_and(|length| length > MAX_DOCUMENT_BYTES)
    {
        return Err(Error::Invalid(
            "This PDF is larger than the 128 MB MVP limit.".into(),
        ));
    }
    let mut bytes = Vec::new();
    request
        .as_reader()
        .take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(Error::Invalid(
            "This PDF is larger than the 128 MB MVP limit.".into(),
        ));
    }
    Ok(bytes)
}

fn json_body<T: serde::de::DeserializeOwned>(request: &mut Request) -> Result<T, Error> {
    serde_json::from_slice(&body(request)?)
        .map_err(|error| Error::Invalid(format!("The preview request is invalid: {error}")))
}

fn handle(request: &mut Request, workspace: &mut Workspace) -> Result<Payload, Error> {
    if request.url() == "/health" && request.method() == &Method::Get {
        return Ok(Payload::Json(
            serde_json::json!({ "status": "ready", "engine": "Rust / lopdf" }),
        ));
    }
    // This dev-only bridge accepts no filesystem paths and requires a custom
    // header, so an unrelated web page cannot submit ambient form requests.
    if header(request, "X-Folio-Preview").as_deref() != Some("1") {
        return Err(Error::Invalid(
            "The local preview header is required.".into(),
        ));
    }
    if let Some(origin) = header(request, "Origin")
        && origin != "http://127.0.0.1:1420"
        && origin != "http://localhost:1420"
    {
        return Err(Error::Invalid("This preview origin is not allowed.".into()));
    }
    let route = request.url().to_owned();
    let info = match (request.method(), route.as_str()) {
        (&Method::Get, "/bootstrap") => workspace.bootstrap()?,
        (&Method::Post, "/demo") => {
            workspace.open_demo(header(request, "X-Discard-Changes").as_deref() == Some("true"))?
        }
        (&Method::Post, "/create") => {
            let input: CreateRequest = json_body(request)?;
            workspace.create_document(input.width, input.height, input.discard_changes)?
        }
        (&Method::Post, "/import") => {
            let document_id = numeric_header(request, "X-Document-Id")?;
            let revision = numeric_header(request, "X-Revision")?;
            let at = usize::try_from(numeric_header(request, "X-Insert-At")?)
                .map_err(|_| Error::Invalid("The insertion position is invalid.".into()))?;
            workspace
                .current_mut(document_id, revision)?
                .import_pages(body(request)?, at)?
        }
        (&Method::Post, "/extract") => {
            let input: ExtractRequest = json_body(request)?;
            return Ok(Payload::Pdf(
                workspace
                    .check_revision(input.document_id, input.revision)?
                    .extract_pages(&input.pages)?,
            ));
        }
        (&Method::Post, "/open") => {
            let name = header(request, "X-File-Name")
                .ok_or_else(|| Error::Invalid("The PDF filename is missing.".into()))?;
            let name = percent_encoding::percent_decode_str(&name)
                .decode_utf8()
                .map_err(|_| Error::Invalid("The filename is not valid UTF-8.".into()))?
                .into_owned();
            if name.len() > 1024 || name.trim().is_empty() {
                return Err(Error::Invalid("The PDF filename is invalid.".into()));
            }
            let discard = header(request, "X-Discard-Changes").as_deref() == Some("true");
            workspace.open_bytes(body(request)?, name, discard)?
        }
        (&Method::Get, "/bytes") => {
            let id = header(request, "X-Document-Id")
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| Error::Invalid("The document identifier is missing.".into()))?;
            let revision = header(request, "X-Revision")
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| Error::Invalid("The document revision is missing.".into()))?;
            return Ok(Payload::Pdf(
                workspace.check_revision(id, revision)?.bytes().to_vec(),
            ));
        }
        (&Method::Post, "/edit") => {
            let input: EditRequest = json_body(request)?;
            workspace
                .current_mut(input.document_id, input.revision)?
                .apply(input.edit)?
        }
        (&Method::Post, "/undo") => {
            let input: Revision = json_body(request)?;
            workspace
                .current_mut(input.document_id, input.revision)?
                .undo()?
        }
        (&Method::Post, "/redo") => {
            let input: Revision = json_body(request)?;
            workspace
                .current_mut(input.document_id, input.revision)?
                .redo()?
        }
        _ => return Err(Error::Invalid("This preview route does not exist.".into())),
    };
    Ok(Payload::Json(serde_json::to_value(info).map_err(
        |error| Error::Invalid(format!("Could not serialize document state: {error}")),
    )?))
}

fn numeric_header(request: &Request, name: &'static str) -> Result<u64, Error> {
    header(request, name)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| Error::Invalid(format!("The {name} request header is missing or invalid.")))
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let server = Server::http("127.0.0.1:19148")?;
    let mut workspace = Workspace::default();
    println!("Folio Rust preview engine ready at http://127.0.0.1:19148");
    for mut request in server.incoming_requests() {
        let (bytes, content_type, status) = match handle(&mut request, &mut workspace) {
            Ok(Payload::Json(value)) => (serde_json::to_vec(&value)?, "application/json", 200),
            Ok(Payload::Pdf(bytes)) => (bytes, "application/pdf", 200),
            Err(error) => {
                eprintln!("Preview request failed: {error}");
                (
                    serde_json::to_vec(&serde_json::json!({ "error": error.to_string() }))?,
                    "application/json",
                    400,
                )
            }
        };
        let response = Response::from_data(bytes)
            .with_status_code(StatusCode(status))
            .with_header(
                Header::from_bytes("Content-Type", content_type)
                    .map_err(|_| "Invalid content header")?,
            )
            .with_header(
                Header::from_bytes("Cache-Control", "no-store")
                    .map_err(|_| "Invalid cache header")?,
            )
            .with_header(
                Header::from_bytes("X-Content-Type-Options", "nosniff")
                    .map_err(|_| "Invalid response header")?,
            );
        if let Err(error) = request.respond(response) {
            eprintln!("Preview response failed: {error}");
        }
    }
    Ok(())
}
