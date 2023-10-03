use axum::body::Body;
use axum::extract::{Path, Query};

use axum::response::Response;
use axum::routing::get;
use axum::{debug_handler, Extension, Router};
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use std::fs;

use std::net::SocketAddr;

use std::sync::{Arc, OnceLock};

use tower_http::services::ServeDir;
use tracing::{debug, info};

static IMG_CACHE: OnceLock<Cache<String, Vec<u8>>> = OnceLock::new();

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;
    dotenvy::dotenv().ok();
    let db = Arc::new(SqlitePool::connect(&std::env::var("DATABASE_URL")?).await?);
    let album_roots = sqlx::query!("SELECT * FROM AlbumRoots")
        .fetch_all(&*db)
        .await
        .unwrap();
    for album_root in &album_roots {
        debug!("Album root: {}", album_root.specificPath.clone().unwrap());
    }
    let _service = ServeDir::new(format!(
        "{}/{}",
        album_roots.first().unwrap().specificPath.clone().unwrap(),
        std::env::var("SUBFOLDER").unwrap()
    ));
    let cache = Cache::builder()
        .name("image_cache")
        .weigher(|_, v: &Vec<u8>| v.len() as u32)
        // Allocate 1/4 of a gigabyte at first and half a gigabyte at most
        .initial_capacity(1024 * 1024 * 1024 / 4)
        .max_capacity(1024 * 1024 * 1024 / 2)
        .time_to_idle(std::time::Duration::from_secs(60 * 30))
        .time_to_live(std::time::Duration::from_secs(60 * 30))
        .build();
    IMG_CACHE.get_or_init(|| cache);
    let app = Router::new()
        .route("/", get(index))
        .route("/image/:file", get(image_serve))
        .layer(Extension(db));
    debug!(
        message = "Starting server",
        addr = std::env::var("ADDR").unwrap()
    );
    axum::Server::bind(&std::env::var("ADDR").unwrap().parse::<SocketAddr>()?)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}

#[debug_handler]
async fn image_serve(
    Path(path): Path<String>,
    Extension(db): Extension<Arc<SqlitePool>>,
) -> Result<Response<Body>, Response<Body>> {
    let cache = IMG_CACHE.get().unwrap();
    let image = cache.get(&path).await;
    let album_root = sqlx::query!("SELECT * FROM AlbumRoots")
        .fetch_all(&*db)
        .await
        .unwrap()
        .first()
        .unwrap()
        .specificPath
        .clone()
        .unwrap();
    let path = format!(
        "{}/{}{}",
        album_root,
        std::env::var("SUBFOLDER").unwrap(),
        path.replace("..", "").replace("//", "/")
    );
    if let Some(image) = image {
        info!("Cache Hit: {}", path);
        Ok(Response::builder()
            .header("Content-Type", "image/jpeg")
            .body(Body::from(image))
            .unwrap())
    } else {
        info!("Serving image: {}", path);
        let image = fs::read(path.clone()).map_err(|_| {
            Response::builder()
                .status(404)
                .body(Body::from(format!("Image not found: {}", path)))
                .unwrap()
        })?;
        cache.insert(path, image.clone()).await;
        Ok(Response::builder()
            .header("Content-Type", "image/jpeg")
            .body(Body::from(image))
            .unwrap())
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Filter {
    filter: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, sqlx::FromRow)]
struct Image {
    name: String,
    tags: Vec<String>,
}

async fn index(
    filter: Option<Query<Filter>>,
    Extension(db): Extension<Arc<SqlitePool>>,
) -> Response<Body> {
    let filter = if let Some(filter) = filter {
        dbg!(filter.0)
    } else {
        Filter {
            filter: String::from("%"),
        }
    };
    info!("Images requested for Filter: {}", filter.filter);
    // Query images that match the filter and end in .JPG
    let images: Vec<SqliteRow> = sqlx::query(
        r#"
    SELECT i.name AS name, GROUP_CONCAT(t.name, ',') AS tags
    FROM Images AS i
    INNER JOIN ImageTags AS it ON i.id = it.imageId
    INNER JOIN Tags AS t ON it.tagId = t.id
    WHERE i.name LIKE $2 AND i.id IN (
        SELECT imageId
        FROM ImageTags AS it
        INNER JOIN Tags AS t ON it.tagId = t.id
        WHERE t.name LIKE $1
    )
    GROUP BY i.id
    "#,
    )
    .bind(filter.filter)
    .bind("%JPG")
    .fetch_all(&*db)
    .await
    .unwrap();

    // Deserialize the concatenated tags string into a Vec<String>
    let images: Vec<Image> = images
        .iter()
        .map(|image| {
            let tags = image
                .get::<String, &str>("tags")
                .split(',')
                .map(|tag| tag.to_string())
                .filter(|tag| {
                    !tag.starts_with("Pick Label")
                        && !tag.starts_with("Scanned for")
                        && !tag.starts_with("Color Label")
                        && !tag.starts_with("Intermediate")
                        && !tag.starts_with("Current Version")
                })
                .collect::<Vec<String>>();
            Image {
                name: image.get::<String, &str>("name"),
                tags,
            }
        })
        .collect();

    let template = liquid::ParserBuilder::with_stdlib()
        .build()
        .unwrap()
        .parse(include_str!("liquid/index.liquid"))
        .unwrap();

    let body = template
        .render(&liquid::object!({
            "images": images,
            "bottom_text": std::env::var("BOTTOM_TEXT").unwrap()
        }))
        .unwrap();

    Response::new(Body::from(body))
}
