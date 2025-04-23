use crate::{
    backend::{Backend, File},
    config::Config,
};
use anyhow::{anyhow, Ok, Result};
use iref::{IriBuf, IriRefBuf};
use time::{format_description::well_known, OffsetDateTime};
use ureq::http::{Method, Request, Uri};
use ureq::Agent;
use xml::name::OwnedName;

pub struct Webdav {
    session: Agent,
    base_iri: Uri,
}

impl Backend for Webdav {
    fn get(&self, key: &str) -> Result<Vec<u8>> {
        Ok(self
            .session
            .get(format!("{}/{}", self.base_iri, key))
            .call()?
            .body_mut()
            .read_to_vec()?)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.session
            .delete(format!("{}/{}", self.base_iri, key))
            .call()?;
        Ok(())
    }

    fn put(&self, key: &str, data: &[u8]) -> Result<()> {
        self.session
            .put(format!("{}/{}", self.base_iri, key))
            .send(data)?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<File>> {
        let response = self.session.run(
            Request::builder()
                .method(Method::from_bytes(b"PROPFIND")?)
                .uri(self.base_iri.clone())
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
                                match href_uri.path().suffix(base_uri.path()) {
                                    Some(thing) => {
                                        // Ignore lines ending with "/" as they are directories
                                        if !thing.clone().to_string().ends_with("/") {
                                            files.push(File {
                                                key: thing.to_string(),
                                                last_modified: lastmodified,
                                            });
                                        }
                                    }
                                    None => println!("I don't know what {:?} is?", href_uri),
                                };
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
        // We need to remove the ending slash as they would conflict with the suffix detection
        // feature of the Iri lib
        let iri = config.url.trim_end_matches("/").to_string();
        let iri = Uri::from_maybe_shared(iri)?;
        let agent_config = Agent::config_builder()
            .allow_non_standard_methods(true)
            .build();
        Ok(Self {
            session: Agent::new_with_config(agent_config),
            base_iri: iri,
        })
    }
}
