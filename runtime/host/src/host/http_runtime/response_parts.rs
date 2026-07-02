use reqwest::header::HeaderMap;

pub(super) struct HttpResponseHead<Headers = HeaderMap> {
    status: u16,
    headers: Headers,
}

impl<Headers> HttpResponseHead<Headers> {
    pub(super) fn new(status: u16, headers: Headers) -> Self {
        Self { status, headers }
    }

    pub(super) fn status(&self) -> u16 {
        self.status
    }

    pub(super) fn headers(&self) -> &Headers {
        &self.headers
    }

    pub(super) fn into_headers(self) -> Headers {
        self.headers
    }

    pub(super) fn is_success_status(&self) -> bool {
        (200..=299).contains(&self.status)
    }
}

impl HttpResponseHead<HeaderMap> {
    pub(super) fn from_response(response: &reqwest::Response) -> Self {
        Self {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
        }
    }
}

pub(super) struct HttpResponseParts<Headers = HeaderMap> {
    head: HttpResponseHead<Headers>,
    body: Vec<u8>,
}

impl<Headers> HttpResponseParts<Headers> {
    pub(super) fn new(head: HttpResponseHead<Headers>, body: Vec<u8>) -> Self {
        Self { head, body }
    }

    pub(super) fn head(&self) -> &HttpResponseHead<Headers> {
        &self.head
    }

    pub(super) fn body(&self) -> &[u8] {
        &self.body
    }

    pub(super) fn into_inner(self) -> (HttpResponseHead<Headers>, Vec<u8>) {
        (self.head, self.body)
    }
}
