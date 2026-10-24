use crate::{
    backend::{Backend, File},
    config::Config,
};
use anyhow::{anyhow, Ok, Result};
use iref::{uri::SegmentBuf, IriBuf, IriRefBuf, UriBuf};
use log::debug;
use pct_str::{PctString, URIReserved};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use time::{format_description::well_known, OffsetDateTime};
use ureq::Agent;
use ureq::{
    http::{Method, Request},
    Error,
};
use xml::name::OwnedName;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtraHeader {
    key: String,
    value: String,
}

pub struct Webdav {
    session: Agent,
    base_uri: UriBuf,
    extra_headers: Vec<ExtraHeader>,
}

struct RemoteEntry {
    uri: IriBuf,
    key: String,
    last_modified: Option<OffsetDateTime>,
    is_collection: bool,
}

fn get_segments(key: &str) -> Vec<String> {
    key.split('/')
        .map(|s| PctString::encode(s.chars(), URIReserved).into_string())
        .collect()
}

impl Webdav {
    fn propfind(&self, collection: &IriBuf) -> Result<Vec<RemoteEntry>> {
        let mut request = Request::builder()
            .method(Method::from_bytes(b"PROPFIND")?)
            .uri(collection.to_string())
            .header("Content-Type", "application/xml")
            .header("Depth", "1");

        for header in &self.extra_headers {
            request = request.header(&header.key, &header.value);
        }
        let response = self.session.run(request.body(
            "<?xml version=\"1.0\" encoding=\"utf-8\" ?> 
  <propfind xmlns=\"DAV:\"> 
    <prop> 
      <getlastmodified/>
    </prop> 
  </propfind>",
        )?)?;
        let body = response.into_body();
        let reader = xml::EventReader::new(body.into_reader());
        let base_iri = IriBuf::new(self.base_uri.to_string())?;
        let mut value = None;
        let mut in_response = false;
        let mut entries = vec![];
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
                        href = None;
                        lastmodified = None;
                        value = None;
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
                                href = value.take();
                            }
                            "getlastmodified" => {
                                lastmodified = Some(OffsetDateTime::parse(
                                    &value.take().ok_or(anyhow!(
                                        "invalid data returned by the server, getlastmodified value missing"
                                    ))?,
                                    &well_known::Rfc2822,
                                )?);
                            }
                            "response" => {
                                let href = href.take().ok_or(anyhow!(
                                    "invalid data returned by the server, href field missing"
                                ))?;
                                let href = IriRefBuf::new(href)?;
                                let href_uri = href.resolved(collection);
                                let thing = href_uri.relative_to(&base_iri).to_string();
                                let thing = PctString::new(thing)?.decode();
                                let is_collection = thing.ends_with("/") || thing.is_empty();
                                entries.push(RemoteEntry {
                                    uri: href_uri,
                                    key: thing.to_string(),
                                    last_modified: lastmodified.take(),
                                    is_collection,
                                });
                                in_response = false;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(entries)
    }
}

impl Backend for Webdav {
    fn get(&self, key: &str) -> Result<Vec<u8>> {
        let segments = get_segments(key);
        debug!(
            "GET key {}, segments {:?}, base_uri: {}",
            key,
            segments,
            self.base_uri.path()
        );
        let mut request = self
            .session
            .get(format!("{}/{}", self.base_uri, segments.join("/")));
        for header in &self.extra_headers {
            request = request.header(&header.key, &header.value);
        }
        Ok(request.call()?.body_mut().read_to_vec()?)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let segments = get_segments(key);
        debug!(
            "DELETE key {}, segments {:?}, base_uri: {}",
            key,
            segments,
            self.base_uri.path()
        );
        let mut request = self
            .session
            .delete(format!("{}/{}", self.base_uri, segments.join("/")));
        for header in &self.extra_headers {
            request = request.header(&header.key, &header.value);
        }
        request.call()?;
        Ok(())
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        let segments: Vec<SegmentBuf> = get_segments(key)
            .iter()
            .map(|s| {
                Ok(SegmentBuf::new(s.as_bytes().to_owned())
                    .map_err(|e| anyhow!("Invalid path name {:?}", e))?)
            })
            .collect::<Result<Vec<SegmentBuf>>>()?;
        debug!(
            "PUT key {}, segments {:?}, base_uri: {}",
            key,
            segments,
            self.base_uri.path()
        );
        let mut tmp = self.base_uri.clone();

        // We need to remove the tailing /
        tmp.path_mut().pop();
        for s in &segments[..segments.len() - 1] {
            debug!("Checking if collection {} exists", s);
            tmp.path_mut().push(s);
            let mut request = Request::builder()
                .method(Method::from_bytes(b"PROPFIND")?)
                .header("Depth", "1") // TODO: Probably related to the webdav compliance
                .uri(tmp.to_string());
            for header in &self.extra_headers {
                request = request.header(&header.key, &header.value);
            }
            let response = self.session.run(request.body(String::new())?);
            if let Err(Error::StatusCode(err_code)) = response {
                if err_code == 404 {
                    debug!("Creating collection {}", tmp);
                    let mut request = Request::builder()
                        .method(Method::from_bytes(b"MKCOL")?)
                        .uri(tmp.to_string());
                    for header in &self.extra_headers {
                        request = request.header(&header.key, &header.value);
                    }
                    self.session.run(request.body(String::new())?)?;
                } else {
                    response?;
                }
            } else {
                response?;
            }
        }
        tmp.path_mut()
            .push(segments.last().ok_or(anyhow!("Empty key"))?);
        let mut request = self.session.put(tmp.to_string());
        for header in &self.extra_headers {
            request = request.header(&header.key, &header.value);
        }
        request.send(data)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<File>> {
        let mut files = vec![];
        let mut collections = vec![IriBuf::new(self.base_uri.to_string())?];
        let mut visited = HashSet::new();

        while let Some(collection) = collections.pop() {
            if !visited.insert(collection.to_string()) {
                continue;
            }
            for entry in self.propfind(&collection)? {
                if entry.is_collection {
                    // The first entry is the collection itself.
                    if !entry.key.is_empty() {
                        collections.push(entry.uri);
                    }
                } else {
                    let last_modified = entry.last_modified.ok_or(anyhow!(
                        "invalid data returned by the server, lastmodified field missing for {}",
                        entry.key
                    ))?;
                    files.push(File {
                        key: entry.key,
                        last_modified,
                    });
                }
            }
        }
        Ok(files)
    }

    fn new(config: &Config) -> Result<Self> {
        // We need to add the ending slash to express that it is a collection
        let uri = if config.url.ends_with('/') {
            config.url.clone()
        } else {
            config.url.clone() + "/"
        };
        let uri = UriBuf::new(uri.clone().into())
            .map_err(|e| anyhow!("Cannot parse URI {}: {:?}", uri, e))?;
        let agent_config = Agent::config_builder()
            .allow_non_standard_methods(true)
            .build();
        Ok(Self {
            session: Agent::new_with_config(agent_config),
            base_uri: uri,
            extra_headers: config.extra_headers.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    // Since the spec is pretty complex and I did not want to reimplement it, the tests are run directly against
    // a compliant implementation based on `warp`, in the crate `webdav_handler`
    // The test server requires a custom header on every request, which checks that
    // the configured extra headers are applied to all WebDAV methods.
    // It also does not allow infinite Depth for PROPFIND, which checks that list()
    // recursively walks collections with Depth: 1.

    use super::Config;
    use super::ExtraHeader;
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
    use warp::Filter;
    use webdav_handler::fakels::FakeLs;
    use webdav_handler::memfs::MemFs;
    use webdav_handler::warp::dav_handler;
    use webdav_handler::DavHandler;

    pub fn dav_server() -> BoxedFilter<(impl Reply,)> {
        let handler = DavHandler::builder()
            .filesystem(MemFs::new())
            .locksystem(FakeLs::new())
            .build_handler();
        warp::header::exact("x-test-header", "yes")
            .and(dav_handler(handler))
            .map(|reply| reply)
            .boxed()
    }

    impl ExtraHeader {
        fn new(key: &str, value: &str) -> Self {
            ExtraHeader {
                key: key.to_string(),
                value: value.to_string(),
            }
        }
    }

    #[tokio::test]
    async fn test_single_file_root() {
        let _ = env_logger::builder().is_test(true).try_init();

        with_server(|port| {
            let c = Config {
                ignore: vec![],
                root_dir: None,
                url: format!("http://localhost:{}", port).to_string(),
                extra_headers: vec![ExtraHeader::new("x-test-header", "yes")],
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
                extra_headers: vec![ExtraHeader::new("x-test-header", "yes")],
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
                extra_headers: vec![ExtraHeader::new("x-test-header", "yes")],
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

    #[tokio::test]
    async fn test_special_char_names() {
        let _ = env_logger::builder().is_test(true).try_init();

        with_server(|port| {
            let c = Config {
                ignore: vec![],
                root_dir: None,
                url: format!("http://localhost:{}", port).to_string(),
                extra_headers: vec![ExtraHeader::new("x-test-header", "yes")],
            };
            let w = Webdav::new(&c)?;
            w.put("/d1/$ !💣.txt", b"hello world")?;
            assert!(w.get("/d1/$ !💣.txt")? == b"hello world");

            let file_list = w.list()?;
            assert!(
                file_list.len() == 1,
                "Expecting 1 file only in file list: {:?}",
                file_list
            );
            assert!(
                file_list[0].key == "d1/$ !💣.txt",
                "Expecting d1/$ !💣.txt only in file list: {:?}",
                file_list
            );

            w.delete("/d1/$ !💣.txt")?;
            assert!(w.list()?.len() == 0);

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
