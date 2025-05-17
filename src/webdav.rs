use crate::{
    backend::{Backend, File},
    config::Config,
};
use anyhow::{anyhow, Ok, Result};
use iref::{IriBuf, IriRefBuf};
use log::{debug, info};
use pct_str::{IriReserved, PctString};
use time::{format_description::well_known, OffsetDateTime};
use ureq::Agent;
use ureq::{
    http::{Method, Request},
    Error,
};
use xml::name::OwnedName;

pub struct Webdav {
    session: Agent,
    base_iri: IriBuf,
}

impl Backend for Webdav {
    fn get(&self, key: &str) -> Result<Vec<u8>> {
        let encoded = PctString::encode(key.chars(), IriReserved::Query);
        Ok(self
            .session
            .get(format!("{}/{}", self.base_iri, encoded))
            .call()?
            .body_mut()
            .read_to_vec()?)
    }
    fn delete(&self, key: &str) -> Result<()> {
        let encoded = PctString::encode(key.chars(), IriReserved::Query);
        self.session
            .delete(format!("{}/{}", self.base_iri, encoded))
            .call()?;
        Ok(())
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let key = IriRefBuf::new(PctString::encode(key.chars(), IriReserved::Query).to_string())?;
        let key_uri = key.resolved(&self.base_iri);
        info!(
            "key {}, key_uri {}, base_uri: {}",
            key,
            key_uri,
            self.base_iri.path()
        );
        let suffix = key_uri.relative_to(&self.base_iri);
        let mut tmp = self.base_iri.clone();

        // We need to remove the tailing /
        tmp.path_mut().pop();
        let segments: Vec<_> = suffix.path().segments().collect();
        for s in &segments[..segments.len() - 1] {
            debug!("Checking if collection {} exists", s);
            tmp.path_mut().push(s);
            let request = Request::builder()
                .method(Method::from_bytes(b"PROPFIND")?)
                .header("Depth", "1") // TODO: Probably related to the webdav compliance
                .uri(tmp.to_string())
                .body(String::new())?;
            let response = self.session.run(request);
            if let Err(Error::StatusCode(err_code)) = response {
                if err_code == 404 {
                    debug!("Creating collection {}", tmp);
                    let request = Request::builder()
                        .method(Method::from_bytes(b"MKCOL")?)
                        .uri(tmp.to_string())
                        .body(String::new())?;
                    self.session.run(request)?;
                } else {
                    response?;
                }
            } else {
                response?;
            }
        }
        tmp.path_mut()
            .push(segments.last().ok_or(anyhow!("Empty key"))?);
        self.session.put(tmp.to_string()).send(data)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<File>> {
        let response = self.session.run(
            Request::builder()
                .method(Method::from_bytes(b"PROPFIND")?)
                .uri(self.base_iri.to_string())
                .header("Content-Type", "application/xml")
                .body(
                    "<?xml version=\"1.0\" encoding=\"utf-8\" ?> 
  <propfind xmlns=\"DAV:\"> 
    <prop> 
      <getlastmodified/>
    </prop>
  </propfind>",
                )?,
        )?;
        let body = response.into_body();
        let reader = xml::EventReader::new(body.into_reader());
        let mut value = None;
        let mut in_response = false;
        let mut files = vec![];
        let mut href = None;
        let mut lastmodified = None;
        for event in reader {
            match event? {
                xml::reader::XmlEvent::StartElement {
                    name:
                        OwnedName {
                            local_name,
                            namespace: _,
                            prefix: _,
                        },
                    attributes: _,
                    namespace: _,
                } => match local_name.as_str() {
                    "href" | "getlastmodified" => {
                        if in_response {
                            value = Some(String::new());
                        }
                    }
                    "response" => {
                        in_response = true;
                    }
                    _ => {}
                },
                xml::reader::XmlEvent::Characters(content) => {
                    if let Some(ref mut value) = value {
                        value.push_str(&content);
                    }
                }
                xml::reader::XmlEvent::EndElement {
                    name:
                        OwnedName {
                            local_name,
                            namespace: _,
                            prefix: _,
                        },
                } => {
                    if in_response {
                        match local_name.as_str() {
                            "href" => {
                                href = value.clone();
                                value = None;
                            }
                            "getlastmodified" => {
                                lastmodified = Some(OffsetDateTime::parse(
                                    &value.unwrap(),
                                    &well_known::Rfc2822,
                                )?);
                                value = None;
                            }
                            "response" => {
                                let href = href.clone().ok_or(anyhow!(
                                    "invalid data returned by the server, href field missing"
                                ))?;
                                let lastmodified = lastmodified.ok_or(anyhow!("invalid data returned by the server, lastmodified field missing"))?;
                                let base_uri = IriBuf::new(self.base_iri.to_string())?;
                                let href = IriRefBuf::new(href)?;
                                let href_uri = href.resolved(&base_uri);
                                let thing = href_uri.relative_to(&base_uri).to_string();
                                // Ignore empty keys and lines ending with "/" as they are directories
                                if !(thing.ends_with("/") || thing.is_empty()) {
                                    files.push(File {
                                        key: thing.to_string(),
                                        last_modified: lastmodified,
                                    });
                                }
                                in_response = false;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(files)
    }

    fn new(config: &Config) -> Result<Self> {
        // We need to add the ending slash to express that it is a collection
        let iri = if config.url.ends_with('/') {
            config.url.clone()
        } else {
            config.url.clone() + "/"
        };
        let iri = IriBuf::new(iri)?;
        let agent_config = Agent::config_builder()
            .allow_non_standard_methods(true)
            .build();
        Ok(Self {
            session: Agent::new_with_config(agent_config),
            base_iri: iri,
        })
    }
}

#[cfg(test)]
mod tests {

    use super::Config;
    use super::Webdav;
    use crate::backend::Backend;
    use anyhow::Result;
    use env_logger;
    use log::{info, warn};
    use std::net::SocketAddr;
    use time::Duration;
    use time::OffsetDateTime;
    use tokio;
    use tokio::task::spawn_blocking;
    use warp::filters::BoxedFilter;
    use warp::reply::Reply;
    use webdav_handler::fakels::FakeLs;
    use webdav_handler::memfs::MemFs;
    use webdav_handler::warp::dav_handler;
    use webdav_handler::DavHandler;

    pub fn dav_server() -> BoxedFilter<(impl Reply,)> {
        let handler = DavHandler::builder()
            .filesystem(MemFs::new())
            .locksystem(FakeLs::new())
            .build_handler();
        dav_handler(handler)
    }

    #[tokio::test]
    async fn test_single_file_root() {
        let _ = env_logger::builder().is_test(true).try_init();

        with_server(|port| {
            let c = Config {
                ignore: vec![],
                root_dir: None,
                url: format!("http://localhost:{}", port).to_string(),
            };
            let w = Webdav::new(&c)?;
            w.put("/root.txt", b"hello world")?;
            assert!(w.get("/root.txt")? == b"hello world");

            let file_list = w.list()?;
            assert!(
                file_list.len() == 1,
                "Expecting /root/txt only in file list: {:?}",
                file_list
            );
            assert!(file_list[0].key == "root.txt");
            assert!(file_list[0].last_modified < OffsetDateTime::now_utc());
            assert!(file_list[0].last_modified > OffsetDateTime::now_utc() - Duration::seconds(10));

            w.delete("/root.txt")?;
            assert!(w.get("/root.txt").is_err());

            let file_list = w.list()?;
            assert!(file_list.len() == 0);

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn test_multiple_file_dirs() {
        let _ = env_logger::builder().is_test(true).try_init();

        with_server(|port| {
            let c = Config {
                ignore: vec![],
                root_dir: None,
                url: format!("http://localhost:{}", port).to_string(),
            };
            let w = Webdav::new(&c)?;
            w.put("/d1/d2/f1.txt", b"hello world")?;
            w.put("/d1/d3/f1.txt", b"hello world2")?;
            w.put("/d1/f1.txt", b"hello world3")?;
            assert!(w.get("/d1/d3/f1.txt")? == b"hello world2");
            assert!(w.get("d1/d3/f1.txt")? == b"hello world2");

            let file_list = w.list()?;
            assert!(
                file_list.len() == 3,
                "Expecting 3 files only in file list: {:?}",
                file_list
            );

            Ok(())
        })
        .await;
    }

    #[tokio::test]
    async fn test_delete_files() {
        let _ = env_logger::builder().is_test(true).try_init();

        with_server(|port| {
            let c = Config {
                ignore: vec![],
                root_dir: None,
                url: format!("http://localhost:{}", port).to_string(),
            };
            let w = Webdav::new(&c)?;
            w.put("/d1/d2/f1.txt", b"hello world")?;
            w.put("/d1/d3/f1.txt", b"hello world2")?;
            w.put("/d1/f1.txt", b"hello world3")?;

            w.delete("/d1/f1.txt")?;
            let file_list = w.list()?;
            assert!(
                file_list.len() == 2,
                "Expecting 2 files only in file list: {:?}",
                file_list
            );

            w.delete("/d1/d2/f1.txt")?;
            let file_list = w.list()?;
            assert!(
                file_list.len() == 1,
                "Expecting /d1/d3/f1.txt only in file list: {:?}",
                file_list
            );
            assert!(
                file_list[0].key == "d1/d3/f1.txt",
                "Expecting d1/d3/f1.txt only in file list: {:?}",
                file_list
            );

            Ok(())
        })
        .await;
    }

    async fn with_server<F>(test: F) -> ()
    where
        F: FnOnce(u16) -> Result<()> + std::marker::Send + 'static,
    {
        let mut port = 4900;
        let (send, server) = loop {
            let (send, rcv) = tokio::sync::oneshot::channel();
            port += 1;
            if port > 5000 {
                assert!(false, "Could not find a port to bind to for the test");
            }

            let addr: SocketAddr = ([127, 0, 0, 1], port).into();
            let warpdav = dav_server();
            let result = warp::serve(warpdav).try_bind_with_graceful_shutdown(addr, async {
                rcv.await.ok();
            });
            match result {
                Err(e) => {
                    warn!("Could not bind to port {}: {}", port, e);
                    continue;
                }
                Ok((_addr, server)) => break (send, server),
            }
        };
        info!("Spawning test server");
        let ts = tokio::spawn(server);
        info!("Running test");
        let result = spawn_blocking(move || test(port)).await;
        result
            .expect("Error when fetching the tests")
            .expect("Error during the test");

        send.send(())
            .expect("Unexpected error when sending termination signal for the webdav server");
        info!("Shutting down test server");
        ts.await
            .expect("Error when shutting down webdav test server");
    }
}
