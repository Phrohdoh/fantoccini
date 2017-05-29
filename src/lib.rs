//! A high-level API for programmatically interacting with web pages through WebDriver.
//!
//! This crate uses the [WebDriver protocol] to drive a conforming (potentially headless) browser
//! through relatively high-level operations such as "click this element", "submit this form", etc.
//!
//! Most interactions are driven by using [CSS selectors]. With most WebDriver-compatible browser
//! being fairly recent, the more expressive levels of the CSS standard are also supported, giving
//! fairly [powerful] [operators].
//!
//! Forms are managed by first calling `Client::form`, and then using the methods on `Form` to
//! manipulate the form's fields and eventually submitting it.
//!
//! For low-level access to the page, `Client::source` can be used to fetch the full page HTML
//! source code, and `Client::raw_client_for` to build a raw HTTP request for a particular URL.
//!
//! # Examples
//!
//! These examples all assume that you have a [WebDriver compatible] process running on port 4444.
//! A quick way to get one is to run [`geckodriver`] at the command line. The code also has
//! partial support for the legacy WebDriver protocol used by `chromedriver` and `ghostdriver`.
//!
//! The examples will be using `unwrap` generously --- you should probably not do that in your
//! code, and instead deal with errors when they occur. This is particularly true for methods that
//! you *expect* might fail, such as lookups by CSS selector.
//!
//! Let's start out clicking around on Wikipedia:
//!
//! ```rust,no_run
//! # use fantoccini::Client;
//! let mut c = Client::new("http://localhost:4444").unwrap();
//! // go to the Wikipedia page for Foobar
//! c.goto("https://en.wikipedia.org/wiki/Foobar").unwrap();
//! assert_eq!(c.current_url().unwrap().as_ref(), "https://en.wikipedia.org/wiki/Foobar");
//! // click "Foo (disambiguation)"
//! c.by_selector(".mw-disambig").unwrap().click().unwrap();
//! // click "Foo Lake"
//! c.by_link_text("Foo Lake").unwrap().click().unwrap();
//! assert_eq!(c.current_url().unwrap().as_ref(), "https://en.wikipedia.org/wiki/Foo_Lake");
//! ```
//!
//! How did we get to the Foobar page in the first place? We did a search!
//! Let's make the program do that for us instead:
//!
//! ```rust,no_run
//! # use fantoccini::Client;
//! # let mut c = Client::new("http://localhost:4444").unwrap();
//! // go to the Wikipedia frontpage this time
//! c.goto("https://www.wikipedia.org/").unwrap();
//! // find, fill out, and submit the search form
//! {
//!     let mut f = c.form("#search-form").unwrap();
//!     f.set_by_name("search", "foobar").unwrap();
//!     f.submit().unwrap();
//! }
//! // we should now have ended up in the rigth place
//! assert_eq!(c.current_url().unwrap().as_ref(), "https://en.wikipedia.org/wiki/Foobar");
//! ```
//!
//! What if we want to download a raw file? Fantoccini has you covered:
//!
//! ```rust,no_run
//! # use fantoccini::Client;
//! # let mut c = Client::new("http://localhost:4444").unwrap();
//! // go back to the frontpage
//! c.goto("https://www.wikipedia.org/").unwrap();
//! // find the source for the Wikipedia globe
//! let img = c.by_selector("img.central-featured-logo")
//!     .expect("image should be on page")
//!     .attr("src")
//!     .unwrap()
//!     .expect("image should have a src");
//! // now build a raw HTTP client request (which also has all current cookies)
//! let raw = c.raw_client_for(fantoccini::Method::Get, &img).unwrap();
//! // this is a RequestBuilder from hyper, so we could also add POST data here
//! // but for this we just send the request
//! let mut res = raw.send().unwrap();
//! // we then read out the image bytes
//! use std::io::prelude::*;
//! let mut pixels = Vec::new();
//! res.read_to_end(&mut pixels).unwrap();
//! // and voilla, we now have the bytes for the Wikipedia logo!
//! assert!(pixels.len() > 0);
//! println!("Wikipedia logo is {}b", pixels.len());
//! ```
//!
//! [WebDriver protocol]: https://www.w3.org/TR/webdriver/
//! [CSS selectors]: https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_Selectors
//! [powerful]: https://developer.mozilla.org/en-US/docs/Web/CSS/Pseudo-classes
//! [operators]: https://developer.mozilla.org/en-US/docs/Web/CSS/Attribute_selectors
//! [WebDriver compatible]: https://github.com/Fyrd/caniuse/issues/2757#issuecomment-304529217
//! [`geckodriver`]: https://github.com/mozilla/geckodriver
#![deny(missing_docs)]

extern crate hyper_native_tls;
extern crate rustc_serialize;
extern crate webdriver;
extern crate cookie;
extern crate hyper;

use webdriver::command::WebDriverCommand;
use webdriver::error::WebDriverError;
use webdriver::error::ErrorStatus;
use webdriver::common::ELEMENT_KEY;
use rustc_serialize::json::Json;
use std::io::prelude::*;

pub use hyper::method::Method;

/// Error types.
pub mod error;

type Cmd = WebDriverCommand<webdriver::command::VoidWebDriverExtensionCommand>;

/// A WebDriver client tied to a single browser session.
pub struct Client {
    c: hyper::Client,
    wdb: hyper::Url,
    session: Option<String>,
    legacy: bool,
    ua: Option<String>,
}

/// A single element on the current page.
pub struct Element<'a> {
    c: &'a mut Client,
    e: webdriver::common::WebElement,
}

/// An HTML form on the current page.
pub struct Form<'a> {
    c: &'a mut Client,
    f: webdriver::common::WebElement,
}

impl Client {
    fn init(&mut self,
            params: webdriver::command::NewSessionParameters)
            -> Result<(), error::NewSessionError> {

        if let webdriver::command::NewSessionParameters::Legacy(..) = params {
            self.legacy = true;
        }

        // Create a new session for this client
        // https://www.w3.org/TR/webdriver/#dfn-new-session
        match self.issue_wd_cmd(WebDriverCommand::NewSession(params)) {
            Ok(Json::Object(mut v)) => {
                // TODO: not all impls are w3c compatible
                // See https://github.com/SeleniumHQ/selenium/blob/242d64ca4cd3523489ac1e58703fd7acd4f10c5a/py/selenium/webdriver/remote/webdriver.py#L189
                // and https://github.com/SeleniumHQ/selenium/blob/242d64ca4cd3523489ac1e58703fd7acd4f10c5a/py/selenium/webdriver/remote/webdriver.py#L200
                if let Some(session_id) = v.remove("sessionId") {
                    if let Some(session_id) = session_id.as_string() {
                        self.session = Some(session_id.to_string());
                        return Ok(());
                    }
                    v.insert("sessionId".to_string(), session_id);
                    Err(error::NewSessionError::NotW3C(Json::Object(v)))
                } else {
                    Err(error::NewSessionError::NotW3C(Json::Object(v)))
                }
            }
            Ok(v) => Err(error::NewSessionError::NotW3C(v)),
            Err(error::CmdError::NotW3C(v)) => Err(error::NewSessionError::NotW3C(v)),
            Err(error::CmdError::NotJson(v)) => {
                Err(error::NewSessionError::NotW3C(Json::String(v)))
            }
            Err(error::CmdError::Standard(e @ WebDriverError {
                                              error: ErrorStatus::SessionNotCreated, ..
                                          })) => Err(error::NewSessionError::SessionNotCreated(e)),
            Err(e) => {
                panic!("unexpected webdriver error; {}", e);
            }
        }
    }

    /// Create a new `Client` associated with a new WebDriver session on the server at the given
    /// URL.
    pub fn new<U: hyper::client::IntoUrl>(webdriver: U) -> Result<Self, error::NewSessionError> {
        // Where is the WebDriver server?
        let wdb = webdriver
            .into_url()
            .map_err(|e| error::NewSessionError::BadWebdriverUrl(e))?;

        // We want tls
        let ssl = hyper_native_tls::NativeTlsClient::new().unwrap();
        let connector = hyper::net::HttpsConnector::new(ssl);
        let client = hyper::Client::with_connector(connector);

        // Set up our WebDriver client
        let mut c = Client {
            c: client,
            wdb,
            session: None,
            legacy: false,
            ua: None,
        };

        // Required capabilities
        // https://www.w3.org/TR/webdriver/#capabilities
        let mut cap = webdriver::capabilities::Capabilities::new();
        //  - we want the browser to wait for the page to load
        cap.insert("pageLoadStrategy".to_string(),
                   Json::String("normal".to_string()));

        let session_config = webdriver::capabilities::SpecNewSessionParameters {
            alwaysMatch: cap.clone(),
            firstMatch: vec![],
        };
        let spec = webdriver::command::NewSessionParameters::Spec(session_config);

        match c.init(spec) {
            Ok(_) => Ok(c),
            Err(error::NewSessionError::NotW3C(json)) => {
                let mut legacy = false;
                match json {
                    Json::String(ref err) if err.starts_with("Missing Command Parameter") => {
                        // ghostdriver
                        legacy = true;
                    }
                    Json::Object(ref err) => {
                        if err.contains_key("message") &&
                           err["message"]
                               .as_string()
                               .map(|s| s.contains("cannot find dict 'desiredCapabilities'"))
                               .unwrap_or(false) {
                            // chromedriver
                            legacy = true;
                        }
                    }
                    _ => {}
                }

                if legacy {
                    // we're dealing with an implementation that only supports the legacy WebDriver
                    // protocol: https://github.com/SeleniumHQ/selenium/wiki/JsonWireProtocol
                    let session_config = webdriver::capabilities::LegacyNewSessionParameters {
                        required: cap,
                        desired: webdriver::capabilities::Capabilities::new(),
                    };
                    let spec = webdriver::command::NewSessionParameters::Legacy(session_config);
                    c.init(spec)?;
                    Ok(c)
                } else {
                    Err(error::NewSessionError::NotW3C(json))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Set the User Agent string to use for all subsequent requests.
    pub fn set_ua<S: Into<String>>(&mut self, ua: S) {
        self.ua = Some(ua.into());
    }

    /// Helper for determining what URL endpoint to use for various requests.
    ///
    /// This mapping is essentially that of https://www.w3.org/TR/webdriver/#list-of-endpoints.
    fn endpoint_for(&self, cmd: &Cmd) -> Result<hyper::Url, hyper::error::ParseError> {
        if let WebDriverCommand::NewSession(..) = *cmd {
            return self.wdb.join("/session");
        }

        let session = self.session.as_ref().unwrap();
        if let WebDriverCommand::DeleteSession = *cmd {
            return self.wdb.join(&format!("/session/{}", session));
        }

        let base = self.wdb.join(&format!("/session/{}/", session))?;
        match *cmd {
            WebDriverCommand::NewSession(..) => unreachable!(),
            WebDriverCommand::DeleteSession => unreachable!(),
            WebDriverCommand::Get(..) |
            WebDriverCommand::GetCurrentUrl => base.join("url"),
            WebDriverCommand::GetPageSource => base.join("source"),
            WebDriverCommand::FindElement(..) => base.join("element"),
            WebDriverCommand::GetCookies => base.join("cookie"),
            WebDriverCommand::ExecuteScript(..) if self.legacy => base.join("execute"),
            WebDriverCommand::ExecuteScript(..) => base.join("execute/sync"),
            WebDriverCommand::GetElementProperty(ref we, ref prop) => {
                base.join(&format!("element/{}/property/{}", we.id, prop))
            }
            WebDriverCommand::GetElementAttribute(ref we, ref attr) => {
                base.join(&format!("element/{}/attribute/{}", we.id, attr))
            }
            WebDriverCommand::FindElementElement(ref p, _) => {
                base.join(&format!("element/{}/element", p.id))
            }
            WebDriverCommand::ElementClick(ref we) => {
                base.join(&format!("element/{}/click", we.id))
            }
            WebDriverCommand::GetElementText(ref we) => {
                base.join(&format!("element/{}/text", we.id))
            }
            WebDriverCommand::ElementSendKeys(ref we, _) => {
                base.join(&format!("element/{}/value", we.id))
            }
            _ => unimplemented!(),
        }
    }

    /// Helper for issuing a WebDriver command, and then reading and parsing the response.
    ///
    /// Since most `WebDriverCommand` arguments already implement `ToJson`, this is mostly a matter
    /// of picking the right URL and method from [the spec], and stuffing the JSON encoded
    /// arguments (if any) into the body.
    ///
    /// [the spec]: https://www.w3.org/TR/webdriver/#list-of-endpoints
    fn issue_wd_cmd(&self,
                    cmd: WebDriverCommand<webdriver::command::VoidWebDriverExtensionCommand>)
                    -> Result<Json, error::CmdError> {
        use rustc_serialize::json::ToJson;
        use hyper::method::Method;
        use webdriver::command;

        // most actions are just get requests with not parameters
        let url = self.endpoint_for(&cmd)?;
        let mut method = Method::Get;
        let mut body = None;

        // but some are special
        match cmd {
            WebDriverCommand::NewSession(command::NewSessionParameters::Spec(ref conf)) => {
                body = Some(format!("{}", conf.to_json()));
                method = Method::Post;
            }
            WebDriverCommand::NewSession(command::NewSessionParameters::Legacy(ref conf)) => {
                body = Some(format!("{}", conf.to_json()));
                method = Method::Post;
            }
            WebDriverCommand::Get(ref params) => {
                body = Some(format!("{}", params.to_json()));
                method = Method::Post;
            }
            WebDriverCommand::FindElement(ref loc) |
            WebDriverCommand::FindElementElement(_, ref loc) => {
                body = Some(format!("{}", loc.to_json()));
                method = Method::Post;
            }
            WebDriverCommand::ExecuteScript(ref script) => {
                body = Some(format!("{}", script.to_json()));
                method = Method::Post;
            }
            WebDriverCommand::ElementSendKeys(_, ref keys) => {
                body = Some(format!("{}", keys.to_json()));
                method = Method::Post;
            }
            WebDriverCommand::ElementClick(..) => {
                body = Some("{}".to_string());
                method = Method::Post;
            }
            WebDriverCommand::DeleteSession => {
                method = Method::Delete;
            }
            _ => {}
        }

        // issue the command to the webdriver server
        let mut res = {
            let mut req = self.c.request(method, url);
            if let Some(ref s) = self.ua {
                req = req.header(hyper::header::UserAgent(s.to_owned()));
            }
            if let Some(ref body) = body {
                let json = body.as_bytes();
                let mut headers = hyper::header::Headers::new();
                headers.set(hyper::header::ContentType::json());
                req.headers(headers)
                    .body(hyper::client::Body::BufBody(json, json.len()))
                    .send()
            } else {
                req.send()
            }
        }?;

        if let WebDriverCommand::ElementClick(..) = cmd {
            // unfortunately implementations seem to sometimes return very eagerly
            use std::thread;
            use std::time::Duration;
            thread::sleep(Duration::from_millis(500));
        }

        // check that the server sent us json
        use hyper::mime::{Mime, TopLevel, SubLevel};
        let ctype = {
            let ctype = res.headers
                .get::<hyper::header::ContentType>()
                .expect("webdriver response did not have a content type");
            (**ctype).clone()
        };
        match ctype {
            Mime(TopLevel::Application, SubLevel::Json, _) => {}
            _ => {
                // nope, something else...
                let mut body = String::new();
                res.read_to_string(&mut body)?;
                return Err(error::CmdError::NotJson(body));
            }
        }

        let is_new_session = if let WebDriverCommand::NewSession(..) = cmd {
            true
        } else {
            false
        };

        // https://www.w3.org/TR/webdriver/#dfn-send-a-response
        // NOTE: the standard specifies that even errors use the "Send a Reponse" steps
        let body = match Json::from_reader(&mut res)? {
            Json::Object(mut v) => {
                if !self.legacy || !is_new_session {
                    v.remove("value")
                        .ok_or_else(|| error::CmdError::NotW3C(Json::Object(v)))
                } else {
                    // legacy implementations do not wrap sessionId inside "value"
                    Ok(Json::Object(v))
                }
            }
            v => Err(error::CmdError::NotW3C(v)),
        }?;

        if res.status.is_success() {
            return Ok(body);
        }

        // https://www.w3.org/TR/webdriver/#dfn-send-an-error
        // https://www.w3.org/TR/webdriver/#handling-errors
        if !body.is_object() {
            return Err(error::CmdError::NotW3C(body));
        }
        let mut body = body.into_object().unwrap();

        // phantomjs injects a *huge* field with the entire screen contents -- remove that
        body.remove("screen");

        if !body.contains_key("error") || !body.contains_key("message") ||
           !body["error"].is_string() || !body["message"].is_string() {
            return Err(error::CmdError::NotW3C(Json::Object(body)));
        }

        use hyper::status::StatusCode;
        let error = body["error"].as_string().unwrap();
        let error = match res.status {
            StatusCode::BadRequest => {
                match error {
                    "element click intercepted" => ErrorStatus::ElementClickIntercepted,
                    "element not selectable" => ErrorStatus::ElementNotSelectable,
                    "element not interactable" => ErrorStatus::ElementNotInteractable,
                    "insecure certificate" => ErrorStatus::InsecureCertificate,
                    "invalid argument" => ErrorStatus::InvalidArgument,
                    "invalid cookie domain" => ErrorStatus::InvalidCookieDomain,
                    "invalid coordinates" => ErrorStatus::InvalidCoordinates,
                    "invalid element state" => ErrorStatus::InvalidElementState,
                    "invalid selector" => ErrorStatus::InvalidSelector,
                    "no such alert" => ErrorStatus::NoSuchAlert,
                    "no such frame" => ErrorStatus::NoSuchFrame,
                    "no such window" => ErrorStatus::NoSuchWindow,
                    "stale element reference" => ErrorStatus::StaleElementReference,
                    _ => unreachable!(),
                }
            }
            StatusCode::NotFound => {
                match error {
                    "unknown command" => ErrorStatus::UnknownCommand,
                    "no such cookie" => ErrorStatus::NoSuchCookie,
                    "invalid session id" => ErrorStatus::InvalidSessionId,
                    "no such element" => ErrorStatus::NoSuchElement,
                    _ => unreachable!(),
                }
            }
            StatusCode::InternalServerError => {
                match error {
                    "javascript error" => ErrorStatus::JavascriptError,
                    "move target out of bounds" => ErrorStatus::MoveTargetOutOfBounds,
                    "session not created" => ErrorStatus::SessionNotCreated,
                    "unable to set cookie" => ErrorStatus::UnableToSetCookie,
                    "unable to capture screen" => ErrorStatus::UnableToCaptureScreen,
                    "unexpected alert open" => ErrorStatus::UnexpectedAlertOpen,
                    "unknown error" => ErrorStatus::UnknownError,
                    "unsupported operation" => ErrorStatus::UnsupportedOperation,
                    _ => unreachable!(),
                }
            }
            StatusCode::RequestTimeout => {
                match error {
                    "timeout" => ErrorStatus::Timeout,
                    "script timeout" => ErrorStatus::ScriptTimeout,
                    _ => unreachable!(),
                }
            }
            StatusCode::MethodNotAllowed => {
                match error {
                    "unknown method" => ErrorStatus::UnknownMethod,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };

        let message = body["message"].as_string().unwrap().to_string();
        Err(WebDriverError::new(error, message).into())
    }

    /// Navigate directly to the given URL.
    pub fn goto<'a>(&'a mut self, url: &str) -> Result<&'a mut Self, error::CmdError> {
        let url = self.current_url()?.join(url)?;
        self.issue_wd_cmd(WebDriverCommand::Get(webdriver::command::GetParameters {
                                                    url: url.into_string(),
                                                }))?;
        Ok(self)
    }

    /// Retrieve the currently active URL for this session.
    pub fn current_url(&self) -> Result<hyper::Url, error::CmdError> {
        let url = self.issue_wd_cmd(WebDriverCommand::GetCurrentUrl)?;
        if let Some(url) = url.as_string() {
            return Ok(hyper::Url::parse(url)?);
        }

        Err(error::CmdError::NotW3C(url))
    }

    /// Get the HTML source for the current page.
    pub fn source(&self) -> Result<String, error::CmdError> {
        let src = self.issue_wd_cmd(WebDriverCommand::GetPageSource)?;
        if let Some(src) = src.as_string() {
            return Ok(src.to_string());
        }

        Err(error::CmdError::NotW3C(src))
    }

    /// Get a `hyper::RequestBuilder` instance with all the same cookies as the current session has
    /// for the given `url`.
    ///
    /// The `RequestBuilder` can then be used to fetch a resource with more granular control (such
    /// as downloading a file).
    ///
    /// Note that the client is tied to the lifetime of the client to prevent the `Client` from
    /// navigating to another page. This is because it would likely be confusing that the builder
    /// did not *also* navigate. Furthermore, the builder's cookies are tied to the URL at the time
    /// of its creation, so after navigation, the user (that's you) may be confused that the right
    /// cookies aren't being included (I know I would).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use fantoccini::Client;
    /// let mut c = Client::new("http://localhost:4444").unwrap();
    /// c.goto("https://www.wikipedia.org/").unwrap();
    /// let img = c.by_selector("img.central-featured-logo").unwrap()
    ///            .attr("src").unwrap().unwrap();
    /// let raw = c.raw_client_for(fantoccini::Method::Get, &img).unwrap();
    /// let mut res = raw.send().unwrap();
    ///
    /// use std::io::prelude::*;
    /// let mut pixels = Vec::new();
    /// res.read_to_end(&mut pixels).unwrap();
    /// println!("Wikipedia logo is {}b", pixels.len());
    /// ```
    pub fn raw_client_for<'a>(&'a mut self,
                              method: Method,
                              url: &str)
                              -> Result<hyper::client::RequestBuilder<'a>, error::CmdError> {
        // We need to do some trickiness here. GetCookies will only give us the cookies for the
        // *current* domain, whereas we want the cookies for `url`'s domain. The fact that cookies
        // can have /path and security constraints makes this even more of a pain. So, to get
        // around all this, we navigate to the URL in question, fetch its cookies, and then
        // navigate back. *Except* that we can't do that either (what if `url` is some huge file?).
        // So we *actually* navigate to some weird url that's deeper than `url`, and hope that we
        // don't end up with a redirect to somewhere entirely different.
        let old_url = self.current_url()?;
        let url = old_url.clone().join(url)?;
        let cookie_url = url.clone().join("please_give_me_your_cookies")?;
        self.goto(&format!("{}", cookie_url))?;
        let cookies = match self.issue_wd_cmd(WebDriverCommand::GetCookies) {
            Ok(cookies) => cookies,
            Err(e) => {
                // go back before we return
                self.goto(&format!("{}", old_url))?;
                return Err(e);
            }
        };
        self.goto(&format!("{}", old_url))?;

        if !cookies.is_array() {
            return Err(error::CmdError::NotW3C(cookies));
        }
        let cookies = cookies.into_array().unwrap();

        // now add all the cookies
        let mut all_ok = true;
        let mut jar = Vec::new();
        for cookie in &cookies {
            if !cookie.is_object() {
                all_ok = false;
                break;
            }

            // https://w3c.github.io/webdriver/webdriver-spec.html#cookies
            let cookie = cookie.as_object().unwrap();
            if !cookie.contains_key("name") || !cookie.contains_key("value") {
                all_ok = false;
                break;
            }

            if !cookie["name"].is_string() || !cookie["value"].is_string() {
                all_ok = false;
                break;
            }

            let val_of = |key| match cookie.get(key) {
                None => webdriver::common::Nullable::Null,
                Some(v) => {
                    if v.is_null() {
                        webdriver::common::Nullable::Null
                    } else {
                        webdriver::common::Nullable::Value(v.clone())
                    }
                }
            };

            let path = val_of("path").map(|v| if let Some(s) = v.as_string() {
                                              s.to_string()
                                          } else {
                                              unimplemented!();
                                          });
            let domain = val_of("domain").map(|v| if let Some(s) = v.as_string() {
                                                  s.to_string()
                                              } else {
                                                  unimplemented!();
                                              });
            let expiry =
                val_of("expiry").map(|v| match v {
                                         Json::U64(secs) => webdriver::common::Date::new(secs),
                                         Json::I64(secs) => {
                                             webdriver::common::Date::new(secs as u64)
                                         }
                                         Json::F64(secs) => {
                                             // this is only needed for chromedriver
                                             webdriver::common::Date::new(secs as u64)
                                         }
                                         _ => unimplemented!(),
                                     });

            // Object({"domain": String("www.wikipedia.org"), "expiry": Null, "httpOnly": Boolean(false), "name": String("CP"), "path": String("/"), "secure": Boolean(false), "value": String("H2")}
            // NOTE: too bad webdriver::response::Cookie doesn't implement FromJson
            let cookie = webdriver::response::Cookie {
                name: cookie["name"].as_string().unwrap().to_string(),
                value: cookie["value"].as_string().unwrap().to_string(),
                path: path,
                domain: domain,
                expiry: expiry,
                secure: cookie
                    .get("secure")
                    .and_then(|v| v.as_boolean())
                    .unwrap_or(false),
                httpOnly: cookie
                    .get("httpOnly")
                    .and_then(|v| v.as_boolean())
                    .unwrap_or(false),
            };

            // so many cookies
            let cookie: cookie::Cookie = cookie.into();
            jar.push(format!("{}", cookie));
        }

        if all_ok {
            let mut headers = hyper::header::Headers::new();
            headers.set(hyper::header::Cookie(jar));
            if let Some(ref s) = self.ua {
                headers.set(hyper::header::UserAgent(s.to_owned()));
            }
            Ok(self.c.request(method, url).headers(headers))
        } else {
            Err(error::CmdError::NotW3C(Json::Array(cookies)))
        }
    }

    /// Find an element by CSS selector.
    pub fn by_selector<'a>(&'a mut self, selector: &str) -> Result<Element<'a>, error::CmdError> {
        let locator = Self::mklocator(selector);
        self.by(locator)
    }

    /// Find an element by its link text.
    ///
    /// The text matching is exact.
    pub fn by_link_text<'a>(&'a mut self, text: &str) -> Result<Element<'a>, error::CmdError> {
        let locator = webdriver::command::LocatorParameters {
            using: webdriver::common::LocatorStrategy::LinkText,
            value: text.to_string(),
        };
        self.by(locator)
    }

    /// Find an element using an XPath expression.
    pub fn by_xpath<'a>(&'a mut self, xpath: &str) -> Result<Element<'a>, error::CmdError> {
        let locator = webdriver::command::LocatorParameters {
            using: webdriver::common::LocatorStrategy::XPath,
            value: xpath.to_string(),
        };
        self.by(locator)
    }

    /// Wait for the given function to return `true` before proceeding.
    ///
    /// This can be useful to wait for something to appear on the page before interacting with it.
    /// While this currently just spins and yields, it may be more efficient than this in the
    /// future. In particular, in time, it may only run `is_ready` again when an event occurs on
    /// the page.
    pub fn wait_for<'a, F>(&'a mut self, mut is_ready: F) -> &'a mut Self
        where F: FnMut(&Client) -> bool
    {
        while !is_ready(self) {
            use std::thread;
            thread::yield_now();
        }
        self
    }

    /// Wait for the page to navigate to a new URL before proceeding.
    ///
    /// If the `current` URL is not provided, `self.current_url()` will be used. Note however that
    /// this introduces a race condition: the browser could finish navigating *before* we call
    /// `current_url()`, which would lead to an eternal wait.
    pub fn wait_for_navigation<'a>(&'a mut self,
                                   current: Option<hyper::Url>)
                                   -> Result<&'a mut Self, error::CmdError> {
        let current = if current.is_none() {
            self.current_url()?
        } else {
            current.unwrap()
        };
        let mut err = None;

        self.wait_for(|c| match c.current_url() {
                          Err(e) => {
                              err = Some(e);
                              true
                          }
                          Ok(ref url) if url == &current => false,
                          Ok(_) => true,
                      });

        if let Some(e) = err { Err(e) } else { Ok(self) }
    }

    /// Locate a form on the page.
    ///
    /// Through the returned `Form`, HTML forms can be filled out and submitted.
    pub fn form<'a>(&'a mut self, selector: &str) -> Result<Form<'a>, error::CmdError> {
        let locator = Self::mklocator(selector);
        let res = self.issue_wd_cmd(WebDriverCommand::FindElement(locator));
        let form = self.parse_lookup(res)?;
        Ok(Form { c: self, f: form })
    }

    // helpers

    fn by<'a>(&'a mut self,
              locator: webdriver::command::LocatorParameters)
              -> Result<Element<'a>, error::CmdError> {
        let res = self.issue_wd_cmd(WebDriverCommand::FindElement(locator));
        let el = self.parse_lookup(res)?;
        Ok(Element { c: self, e: el })
    }

    /// Extract the `WebElement` from a `FindElement` or `FindElementElement` command.
    fn parse_lookup(&self,
                    res: Result<Json, error::CmdError>)
                    -> Result<webdriver::common::WebElement, error::CmdError> {
        let res = res?;
        if !res.is_object() {
            return Err(error::CmdError::NotW3C(res));
        }

        // legacy protocol uses "ELEMENT" as identifier
        let key = if self.legacy { "ELEMENT" } else { ELEMENT_KEY };

        let mut res = res.into_object().unwrap();
        if !res.contains_key(key) {
            return Err(error::CmdError::NotW3C(Json::Object(res)));
        }

        match res.remove(key) {
            Some(Json::String(wei)) => {
                return Ok(webdriver::common::WebElement::new(wei));
            }
            Some(v) => {
                res.insert(key.to_string(), v);
            }
            None => {}
        }

        Err(error::CmdError::NotW3C(Json::Object(res)))
    }

    fn fixup_elements(&self, args: &mut [Json]) {
        if self.legacy {
            for arg in args {
                // the serialization of WebElement uses the W3C index,
                // but legacy implementations need us to use the "ELEMENT" index
                if let Json::Object(ref mut o) = *arg {
                    if let Some(wei) = o.remove(ELEMENT_KEY) {
                        o.insert("ELEMENT".to_string(), wei);
                    }
                }
            }
        }
    }

    /// Make a WebDriver locator for the given CSS selector.
    ///
    /// See https://www.w3.org/TR/webdriver/#element-retrieval.
    fn mklocator(selector: &str) -> webdriver::command::LocatorParameters {
        webdriver::command::LocatorParameters {
            using: webdriver::common::LocatorStrategy::CSSSelector,
            value: selector.to_string(),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.session.is_some() {
            self.issue_wd_cmd(WebDriverCommand::DeleteSession).unwrap();
        }
    }
}

impl<'a> Element<'a> {
    /// Look up an [attribute] value for this element by name.
    ///
    /// `Ok(None)` is returned if the element does not have the given attribute.
    ///
    /// [attribute]: https://dom.spec.whatwg.org/#concept-attribute
    pub fn attr(&self, attribute: &str) -> Result<Option<String>, error::CmdError> {
        let cmd = WebDriverCommand::GetElementAttribute(self.e.clone(), attribute.to_string());
        match self.c.issue_wd_cmd(cmd)? {
            Json::String(v) => Ok(Some(v)),
            Json::Null => Ok(None),
            v => Err(error::CmdError::NotW3C(v)),
        }
    }

    /// Look up a DOM [property] for this element by name.
    ///
    /// `Ok(None)` is returned if the element does not have the given property.
    ///
    /// [property]: https://www.ecma-international.org/ecma-262/5.1/#sec-8.12.1
    pub fn prop(&self, prop: &str) -> Result<Option<String>, error::CmdError> {
        let cmd = WebDriverCommand::GetElementProperty(self.e.clone(), prop.to_string());
        match self.c.issue_wd_cmd(cmd)? {
            Json::String(v) => Ok(Some(v)),
            Json::Null => Ok(None),
            v => Err(error::CmdError::NotW3C(v)),
        }
    }

    /// Retrieve the text contents of this elment.
    pub fn text(&self) -> Result<String, error::CmdError> {
        let cmd = WebDriverCommand::GetElementText(self.e.clone());
        match self.c.issue_wd_cmd(cmd)? {
            Json::String(v) => Ok(v),
            v => Err(error::CmdError::NotW3C(v)),
        }
    }

    /// Retrieve the HTML contents of this element.
    ///
    /// `inner` dictates whether the wrapping node's HTML is excluded or not. For example, take the
    /// HTML:
    ///
    /// ```html
    /// <div id="foo"><hr /></div>
    /// ```
    ///
    /// With `inner = true`, `<hr />` would be returned. With `inner = false`,
    /// `<div id="foo"><hr /></div>` would be returned instead.
    pub fn html(&self, inner: bool) -> Result<String, error::CmdError> {
        let prop = if inner { "innerHTML" } else { "outerHTML" };
        self.prop(prop).map(|v| v.unwrap())
    }

    /// Simulate the user clicking on this element.
    ///
    /// Note that since this *may* result in navigation, we give up the handle to the element.
    pub fn click(mut self) -> Result<&'a mut Client, error::CmdError> {
        let cmd = WebDriverCommand::ElementClick(self.e);
        let r = self.c.issue_wd_cmd(cmd)?;
        if r.is_null() {
            Ok(self.c)
        } else if r.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            // geckodriver returns {} :(
            Ok(self.c)
        } else {
            Err(error::CmdError::NotW3C(r))
        }
    }

    /// Follow the `href` target of the element matching the given CSS selector *without* causing a
    /// click interaction.
    ///
    /// Note that since this *may* result in navigation, we give up the handle to the element.
    pub fn follow(mut self) -> Result<&'a mut Client, error::CmdError> {
        let cmd = WebDriverCommand::GetElementAttribute(self.e, "href".to_string());
        let href = match self.c.issue_wd_cmd(cmd)? {
            Json::String(v) => Ok(v),
            Json::Null => {
                let e = WebDriverError::new(webdriver::error::ErrorStatus::InvalidArgument,
                                            "cannot follow element without href attribute");
                Err(error::CmdError::Standard(e))
            }
            v => Err(error::CmdError::NotW3C(v)),
        }?;
        let url = self.c.current_url()?;
        let href = url.join(&href)?;

        self.c.goto(&format!("{}", href))?;
        Ok(self.c)
    }
}

impl<'a> Form<'a> {
    /// Set the `value` of the given `field` in this form.
    pub fn set_by_name<'s>(&'s mut self,
                           field: &str,
                           value: &str)
                           -> Result<&'s mut Self, error::CmdError> {
        let locator = Client::mklocator(&format!("input[name='{}']", field));
        let locator = WebDriverCommand::FindElementElement(self.f.clone(), locator);
        let res = self.c.issue_wd_cmd(locator);
        let field = self.c.parse_lookup(res)?;

        use rustc_serialize::json::ToJson;
        let mut args = vec![field.to_json(), Json::String(value.to_string())];
        self.c.fixup_elements(&mut args);
        let cmd = webdriver::command::JavascriptCommandParameters {
            script: "arguments[0].value = arguments[1]".to_string(),
            args: webdriver::common::Nullable::Value(args),
        };

        let res = self.c.issue_wd_cmd(WebDriverCommand::ExecuteScript(cmd))?;

        if res.is_null() {
            Ok(self)
        } else {
            Err(error::CmdError::NotW3C(res))
        }
    }

    /// Submit this form using the first available submit button.
    ///
    /// `false` is returned if no submit button was not found.
    pub fn submit(self) -> Result<&'a mut Client, error::CmdError> {
        self.submit_with("input[type=submit],button[type=submit]")
    }

    /// Submit this form using the button matched by the given CSS selector.
    ///
    /// `false` is returned if a matching button was not found.
    pub fn submit_with(self, button: &str) -> Result<&'a mut Client, error::CmdError> {
        let locator = Client::mklocator(button);
        let locator = WebDriverCommand::FindElementElement(self.f, locator);
        let res = self.c.issue_wd_cmd(locator);

        let submit = self.c.parse_lookup(res)?;
        let res = self.c.issue_wd_cmd(WebDriverCommand::ElementClick(submit))?;

        if res.is_null() {
            Ok(self.c)
        } else if res.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            // geckodriver returns {} :(
            Ok(self.c)
        } else {
            Err(error::CmdError::NotW3C(res))
        }
    }

    /// Submit this form using the form submit button with the given label (case-insensitive).
    ///
    /// `false` is returned if a matching button was not found.
    pub fn submit_using(self, button_label: &str) -> Result<&'a mut Client, error::CmdError> {
        let escaped = button_label.replace('\\', "\\\\").replace('"', "\\\"");
        self.submit_with(&format!("input[type=submit][value=\"{}\" i],\
                                  button[type=submit][value=\"{}\" i]",
                                  escaped,
                                  escaped))
    }

    /// Submit this form directly, without clicking any buttons.
    ///
    /// This can be useful to bypass forms that perform various magic when the submit button is
    /// clicked, or that hijack click events altogether (yes, I'm looking at you online
    /// advertisement code).
    ///
    /// Note that since no button is actually clicked, the `name=value` pair for the submit button
    /// will not be submitted. This can be circumvented by using `submit_sneaky` instead.
    pub fn submit_direct(self) -> Result<&'a mut Client, error::CmdError> {
        use rustc_serialize::json::ToJson;

        let mut args = vec![self.f.clone().to_json()];
        self.c.fixup_elements(&mut args);
        let cmd = webdriver::command::JavascriptCommandParameters {
            script: "arguments[0].submit()".to_string(),
            args: webdriver::common::Nullable::Value(args),
        };

        let res = self.c.issue_wd_cmd(WebDriverCommand::ExecuteScript(cmd))?;

        // unfortunately implementations seem to sometimes return very eagerly
        use std::thread;
        use std::time::Duration;
        thread::sleep(Duration::from_millis(500));

        if res.is_null() {
            Ok(self.c)
        } else if res.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            // geckodriver returns {} :(
            Ok(self.c)
        } else {
            Err(error::CmdError::NotW3C(res))
        }
    }

    /// Submit this form directly, without clicking any buttons, and with an extra field.
    ///
    /// Like `submit_direct`, this method will submit this form without clicking a submit button.
    /// However, it will *also* inject a hidden input element on the page that carries the given
    /// `field=value` mapping. This allows you to emulate the form data as it would have been *if*
    /// the submit button was indeed clicked.
    pub fn submit_sneaky(self,
                         field: &str,
                         value: &str)
                         -> Result<&'a mut Client, error::CmdError> {
        use rustc_serialize::json::ToJson;
        let args = vec![self.f.clone().to_json(),
                        Json::String(field.to_string()),
                        Json::String(value.to_string())];
        let cmd = webdriver::command::JavascriptCommandParameters {
            script: "\
                var h = document.createElement('input');\
                h.setAttribute('type', 'hidden');\
                h.setAttribute('name', arguments[1]);\
                h.value = arguments[2];\
                arguments[0].appendChild(h)"
                    .to_string(),
            args: webdriver::common::Nullable::Value(args),
        };

        let res = self.c.issue_wd_cmd(WebDriverCommand::ExecuteScript(cmd))?;

        if res.is_null() {
            self.submit_direct()
        } else if res.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            // geckodriver returns {} :(
            self.submit_direct()
        } else {
            return Err(error::CmdError::NotW3C(res));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tester<F>(f: F)
        where F: FnOnce(Client) -> Result<(), error::CmdError>
    {
        match Client::new("http://localhost:4444") {
            Ok(c) => {
                match f(c) {
                    Ok(_) => {}
                    Err(e) => {
                        println!("{}", e);
                        assert!(false);
                    }
                }
            }
            Err(e) => {
                println!("{}", e);
                assert!(false);
            }
        }
    }

    fn works_inner(mut c: Client) -> Result<(), error::CmdError> {
        // go to the Wikipedia page for Foobar
        c.goto("https://en.wikipedia.org/wiki/Foobar")?;
        assert_eq!(c.current_url()?.as_ref(),
                   "https://en.wikipedia.org/wiki/Foobar");
        // click "Foo (disambiguation)"
        c.by_selector(".mw-disambig")?.click()?;
        // click "Foo Lake"
        c.by_link_text("Foo Lake")?.click()?;
        assert_eq!(c.current_url()?.as_ref(),
                   "https://en.wikipedia.org/wiki/Foo_Lake");
        Ok(())
    }

    #[test]
    #[ignore]
    fn it_works() {
        tester(works_inner)
    }

    fn clicks_inner(mut c: Client) -> Result<(), error::CmdError> {
        // go to the Wikipedia frontpage this time
        c.goto("https://www.wikipedia.org/")?;
        // find, fill out, and submit the search form
        {
            let mut f = c.form("#search-form")?;
            f.set_by_name("search", "foobar")?;
            f.submit()?;
        }
        // we should now have ended up in the rigth place
        assert_eq!(c.current_url()?.as_ref(),
                   "https://en.wikipedia.org/wiki/Foobar");
        Ok(())
    }

    #[test]
    #[ignore]
    fn it_clicks() {
        tester(clicks_inner)
    }

    fn raw_inner(mut c: Client) -> Result<(), error::CmdError> {
        // go back to the frontpage
        c.goto("https://www.wikipedia.org/")?;
        // find the source for the Wikipedia globe
        let img = c.by_selector("img.central-featured-logo")?
            .attr("src")?
            .expect("image should have a src");
        // now build a raw HTTP client request (which also has all current cookies)
        let raw = c.raw_client_for(Method::Get, &img)?;
        // this is a RequestBuilder from hyper, so we could also add POST data here
        // but for this we just send the request
        let mut res = raw.send()?;
        // we then read out the image bytes
        use std::io::prelude::*;
        let mut pixels = Vec::new();
        res.read_to_end(&mut pixels)?;
        // and voilla, we now have the bytes for the Wikipedia logo!
        assert!(pixels.len() > 0);
        println!("Wikipedia logo is {}b", pixels.len());
        Ok(())
    }

    #[test]
    #[ignore]
    fn it_can_be_raw() {
        tester(raw_inner)
    }
}
