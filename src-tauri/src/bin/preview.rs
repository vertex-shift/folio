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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DocumentRequest {
    document_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CloseRequest {
    document_id: u64,
    revision: u64,
    discard_changes: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DraftRequest {
    document_id: u64,
    pending: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtractRequest {
    document_id: u64,
    revision: u64,
    pages: Vec<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TextRunsRequest {
    document_id: u64,
    revision: u64,
    page: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RedactionRequest {
    document_id: u64,
    revision: u64,
    dpi: u16,
    regions: Vec<folio_core::redaction::Region>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RedactionJob {
    job_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnlockRequest {
    request_id: u64,
    password: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenRequest {
    request_id: u64,
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

fn json(outcome: impl serde::Serialize) -> Result<Payload, Error> {
    Ok(Payload::Json(serde_json::to_value(outcome).map_err(
        |_| Error::Invalid("Could not serialize the opening result.".into()),
    )?))
}

fn file_name(request: &Request) -> Result<String, Error> {
    let name = header(request, "X-File-Name")
        .ok_or_else(|| Error::Invalid("The PDF filename is missing.".into()))?;
    let name = percent_encoding::percent_decode_str(&name)
        .decode_utf8()
        .map_err(|_| Error::Invalid("The filename is not valid UTF-8.".into()))?
        .into_owned();
    if name.len() > 1024 || name.trim().is_empty() {
        return Err(Error::Invalid("The PDF filename is invalid.".into()));
    }
    Ok(name)
}

fn handle(
    request: &mut Request,
    workspace: &mut Workspace,
    frontend_port: u16,
) -> Result<Payload, Error> {
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
        && origin != format!("http://127.0.0.1:{frontend_port}")
        && origin != format!("http://localhost:{frontend_port}")
    {
        return Err(Error::Invalid("This preview origin is not allowed.".into()));
    }
    let route = request.url().to_owned();
    let info = match (request.method(), route.as_str()) {
        (&Method::Get, "/bootstrap") => return json(workspace.snapshot()?),
        (&Method::Post, "/demo") => workspace.open_demo()?,
        (&Method::Post, "/create") => {
            let input: CreateRequest = json_body(request)?;
            workspace.create_document(input.width, input.height)?
        }
        (&Method::Post, "/activate") => {
            let input: DocumentRequest = json_body(request)?;
            workspace.activate(input.document_id)?
        }
        (&Method::Post, "/close") => {
            let input: CloseRequest = json_body(request)?;
            return json(workspace.close_document(
                input.document_id,
                input.revision,
                input.discard_changes,
            )?);
        }
        (&Method::Post, "/draft") => {
            let input: DraftRequest = json_body(request)?;
            workspace.set_draft(input.document_id, input.pending)?;
            return Ok(Payload::Json(serde_json::Value::Null));
        }
        (&Method::Post, "/import") => {
            let document_id = numeric_header(request, "X-Document-Id")?;
            let revision = numeric_header(request, "X-Revision")?;
            let at = usize::try_from(numeric_header(request, "X-Insert-At")?)
                .map_err(|_| Error::Invalid("The insertion position is invalid.".into()))?;
            let name = file_name(request)?;
            return json(workspace.request_import_bytes(
                document_id,
                revision,
                at,
                body(request)?,
                name,
            )?);
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
            let name = file_name(request)?;
            return json(workspace.request_open_bytes(body(request)?, name)?);
        }
        (&Method::Post, "/unlock") => {
            let input: UnlockRequest = serde_json::from_slice(&body(request)?)
                .map_err(|_| Error::Invalid("The password request is invalid.".into()))?;
            return json(workspace.unlock_document(input.request_id, &input.password)?);
        }
        (&Method::Post, "/open/cancel") => {
            let input: OpenRequest = json_body(request)?;
            return Ok(Payload::Json(serde_json::json!(
                workspace.cancel_open(input.request_id)
            )));
        }
        (&Method::Get, "/bytes" | "/render-bytes") => {
            let id = header(request, "X-Document-Id")
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| Error::Invalid("The document identifier is missing.".into()))?;
            let revision = header(request, "X-Revision")
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| Error::Invalid("The document revision is missing.".into()))?;
            let session = workspace.check_revision(id, revision)?;
            return Ok(Payload::Pdf(if route == "/render-bytes" {
                session.render_bytes()?
            } else {
                session.bytes().to_vec()
            }));
        }
        (&Method::Post, "/text-runs") => {
            let input: TextRunsRequest = json_body(request)?;
            let runs = workspace
                .check_revision(input.document_id, input.revision)?
                .text_runs(input.page)?;
            return Ok(Payload::Json(serde_json::to_value(runs).map_err(
                |error| Error::Invalid(format!("Could not serialize source text: {error}")),
            )?));
        }
        (&Method::Post, "/redaction/begin") => {
            let input: RedactionRequest = json_body(request)?;
            let plan = workspace.begin_redaction(
                input.document_id,
                input.revision,
                input.dpi,
                &input.regions,
            )?;
            return Ok(Payload::Json(serde_json::to_value(plan).map_err(
                |error| Error::Invalid(format!("Could not serialize the redaction plan: {error}")),
            )?));
        }
        (&Method::Post, "/redaction/page") => {
            let job_id = numeric_header(request, "X-Folio-Job-Id")?;
            let page = usize::try_from(numeric_header(request, "X-Folio-Page")?)
                .map_err(|_| Error::Invalid("The redaction page index is invalid.".into()))?;
            let limit = folio_core::redaction::MAX_PAGE_PNG_BYTES;
            if request.body_length().is_some_and(|length| length > limit) {
                return Err(Error::Invalid(
                    "A rendered page exceeds the 64 MiB PNG limit.".into(),
                ));
            }
            let mut png = Vec::new();
            request
                .as_reader()
                .take(limit as u64 + 1)
                .read_to_end(&mut png)?;
            workspace.stage_redaction_page(job_id, page, &png)?;
            return Ok(Payload::Json(serde_json::Value::Null));
        }
        (&Method::Post, "/redaction/export") => {
            let input: RedactionJob = json_body(request)?;
            return Ok(Payload::Pdf(workspace.redaction_bytes(input.job_id)?));
        }
        (&Method::Post, "/redaction/cancel") => {
            let input: RedactionJob = json_body(request)?;
            return Ok(Payload::Json(serde_json::json!(
                workspace.cancel_redaction(input.job_id)
            )));
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

fn local_port(name: &str, fallback: u16) -> Result<u16, Error> {
    let value = match std::env::var(name) {
        Ok(value) => value
            .parse::<u16>()
            .map_err(|_| Error::Invalid(format!("{name} is not a valid port.")))?,
        Err(std::env::VarError::NotPresent) => return Ok(fallback),
        Err(error) => return Err(Error::Invalid(format!("{name} could not be read: {error}"))),
    };
    if value < 1024 {
        return Err(Error::Invalid(format!(
            "{name} must be from 1024 to 65535."
        )));
    }
    Ok(value)
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port = local_port("FOLIO_PREVIEW_PORT", 19148)?;
    let frontend_port = local_port("FOLIO_DEV_PORT", 1420)?;
    let server = Server::http(("127.0.0.1", port))?;
    let mut workspace = Workspace::default();
    println!("Folio Rust preview engine ready at http://127.0.0.1:{port}");
    for mut request in server.incoming_requests() {
        let (bytes, content_type, status) =
            match handle(&mut request, &mut workspace, frontend_port) {
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
