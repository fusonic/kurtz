use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use kurtz::{config::Config, db::Db, router, App};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

type BoxError = Box<dyn std::error::Error>;

fn port() -> Result<u16, BoxError> {
    Ok(kurtz::config::port(std::env::var("PORT").ok().as_deref())?)
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), BoxError> {
    match std::env::args().nth(1).as_deref() {
        Some("healthcheck") => return healthcheck().await,
        Some("--version") => {
            println!("kurtz {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {}
    }

    // Logging comes first so configuration errors are reported properly.
    let level = kurtz::config::log_level(std::env::var("LOG_LEVEL").ok().as_deref())?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(format!("kurtz={level},warn")))
        .init();

    let cfg = Config::from_env().map_err(|e| format!("configuration error: {e}"))?;
    let port = port().map_err(|e| format!("configuration error: {e}"))?;

    let mut attempt = 0;
    let db = loop {
        match Db::connect(&cfg.database_url).await {
            Ok(db) => break db,
            Err(e) if attempt < 30 => {
                attempt += 1;
                tracing::warn!("database not reachable yet ({e}), retrying in 1 s");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(format!("could not connect to the database: {e}").into()),
        }
    };
    // Migrations always run; they are idempotent and versioned.
    db.migrate().await.map_err(|e| format!("migrations failed: {e}"))?;
    tracing::info!("database ready ({})", db.backend());

    let state = App::new(cfg, db);
    App::spawn_flusher(state.clone());

    let (listener, bind) = bind_listener(port).await?;
    tracing::info!("listening on {bind}");
    axum::serve(listener, router(state.clone()))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    match state.flush_last_used().await {
        Ok(n) if n > 0 => tracing::info!("wrote {n} pending last-used dates"),
        Ok(_) => {}
        Err(e) => tracing::error!("could not write pending last-used dates: {e}"),
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}

/// Listens on all interfaces. `[::]` also accepts IPv4 on a default Linux
/// setup and is the only option in IPv6-only clusters; where IPv6 is disabled
/// (e.g. a plain Docker network) it falls back to `0.0.0.0`.
async fn bind_listener(port: u16) -> Result<(TcpListener, SocketAddr), BoxError> {
    let v6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    if let Ok(listener) = TcpListener::bind(v6).await {
        return Ok((listener, v6));
    }
    let v4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    let listener = TcpListener::bind(v4)
        .await
        .map_err(|e| format!("could not bind {v4}: {e}"))?;
    Ok((listener, v4))
}

/// `kurtz healthcheck`: used by the Docker HEALTHCHECK (the image has no curl).
async fn healthcheck() -> Result<(), BoxError> {
    let port = port()?;
    let mut stream = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| "timeout")??;
    stream
        .write_all(b"GET /_health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await?;
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut buf))
        .await
        .map_err(|_| "timeout")??;
    if buf.starts_with(b"HTTP/1.1 200") || buf.starts_with(b"HTTP/1.0 200") {
        Ok(())
    } else {
        Err("unhealthy".into())
    }
}
