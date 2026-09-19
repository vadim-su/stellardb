//! UI static file serving (when `ui` feature is enabled)

#[cfg(feature = "ui")]
mod embedded {
    use axum::{
        body::Body,
        extract::Path,
        http::{Response, StatusCode, header},
        response::{IntoResponse, Redirect},
    };
    use rust_embed::Embed;

    #[derive(Embed)]
    #[folder = "station/dist"]
    struct Assets;

    /// Serve the main index.html
    pub async fn index_handler() -> impl IntoResponse {
        serve_file("index.html")
    }

    /// Serve static files from /_station/*
    pub async fn static_handler(Path(path): Path<String>) -> impl IntoResponse {
        // If path is empty or doesn't exist, serve index.html (SPA routing)
        let path = if path.is_empty() { "index.html" } else { &path };

        if Assets::get(path).is_some() {
            serve_file(path)
        } else {
            // SPA fallback - serve index.html for client-side routing
            serve_file("index.html")
        }
    }

    /// Redirect root to the canonical Station URL.
    pub async fn root_redirect() -> Redirect {
        Redirect::permanent("/_station/")
    }

    fn serve_file(path: &str) -> Response<Body> {
        match Assets::get(path) {
            Some(content) => {
                let mime = mime_guess::from_path(path).first_or_octet_stream();
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime.as_ref())
                    .body(Body::from(content.data.to_vec()))
                    .expect("valid HTTP response")
            }
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not Found"))
                .expect("valid HTTP response"),
        }
    }
}

#[cfg(feature = "ui")]
pub use embedded::{index_handler, root_redirect, static_handler};
