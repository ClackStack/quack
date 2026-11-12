#!/usr/bin/env python3
"""Load linked pull or merge requests from explicitly configured forge APIs."""

import argparse
import base64
import json
import os
import re
import sys
import ssl
from urllib.error import HTTPError, URLError
from urllib.parse import quote, unquote, urlsplit
from urllib.request import HTTPRedirectHandler, HTTPSHandler, ProxyHandler, Request, build_opener
from xml.sax.saxutils import escape, quoteattr

ROUTES = {
    "github": r"([^/]+/[^/]+)/pull/([0-9]+)",
    "gitlab": r"(.+?)/-/merge_requests/([0-9]+)",
    "gitea": r"([^/]+/[^/]+)/pulls/([0-9]+)",
    "bitbucket": r"([^/]+/[^/]+)/pull-requests/([0-9]+)",
    "bitbucket-server": r"projects/([^/]+/repos/[^/]+)/pull-requests/([0-9]+)",
}
DIFF_WARN_BYTES = 9000
METADATA_BYTES = 1024 * 1024
MAX_REVIEWS = 2


def origin(url):
    if url.scheme not in ("http", "https") or not url.hostname or url.username is not None:
        raise ValueError("expected an HTTP(S) URL without credentials")
    return url.scheme, url.hostname, url.port or (443 if url.scheme == "https" else 80)


def base(value):
    url = urlsplit(value.rstrip("/"))
    origin(url)
    if url.query or url.fragment:
        raise ValueError("forge bases must omit query strings and fragments")
    return url


class SameOriginRedirect(HTTPRedirectHandler):
    def __init__(self, api=None):
        super().__init__()
        self.api = base(api) if api else None

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # urllib otherwise forwards custom auth headers across origins.
        if origin(urlsplit(req.full_url)) != origin(urlsplit(newurl)):
            raise ValueError("API redirected to a different origin")
        if self.api and not urlsplit(newurl).path.startswith(self.api.path + "/"):
            raise ValueError("API redirected outside its configured prefix")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def review_endpoint(link, kind, web, api):
    url = urlsplit(link.rstrip(").,;!?]}"))
    if origin(url) != origin(web) or not url.path.startswith(web.path + "/"):
        return None
    match = re.fullmatch(ROUTES[kind] + r"(?:/.*)?", url.path[len(web.path) + 1:])
    if not match:
        return None
    repository, number = match.groups()
    parts = [unquote(part) for part in repository.split("/")]
    if len(parts) < 2 or any(
        not part or part in (".", "..") or "/" in part or "\\" in part for part in parts
    ):
        return None
    project = "/".join(quote(part, safe="") for part in parts)
    number = number.lstrip("0")
    if not number:
        return None
    routes = {
        "github": f"repos/{project}/pulls/{number}",
        "gitlab": f"projects/{quote('/'.join(parts), safe='')}/merge_requests/{number}",
        "gitea": f"repos/{project}/pulls/{number}",
        "bitbucket": f"repositories/{project}/pullrequests/{number}",
        "bitbucket-server": f"projects/{project}/pull-requests/{number}",
    }
    return f"{api.geturl()}/{routes[kind]}"


def auth_headers(kind, api, variable, user=None, mode=None):
    if mode == "public":
        return {}
    token = os.environ.get(variable) if variable != "-" else None
    if not token:
        if mode in ("token", "basic"):
            raise ValueError("credential unavailable")
        return {}
    if api.scheme != "https" and api.hostname not in ("127.0.0.1", "::1", "localhost"):
        raise ValueError("configured token requires an HTTPS API base")
    if mode == "basic":
        if not user:
            raise ValueError("account email is required")
        pair = base64.b64encode(f"{user}:{token}".encode()).decode("ascii")
        return {"Authorization": f"Basic {pair}"}
    if kind == "gitlab":
        return {"PRIVATE-TOKEN": token}
    if kind == "bitbucket" and (user := os.environ.get(variable + "_USER")):
        pair = base64.b64encode(f"{user}:{token}".encode()).decode("ascii")
        return {"Authorization": f"Basic {pair}"}
    scheme = "token" if kind == "gitea" else "Bearer"
    return {"Authorization": f"{scheme} {token}"}


def read(http, url, headers, accept, limit):
    request = Request(url, headers={**headers, "Accept": accept, "User-Agent": "quack-forge-review"})
    with http.open(request, timeout=getattr(http, "request_timeout", 5)) as response:
        if response.headers.get_content_type() == "text/html":
            raise ValueError("the API returned an HTML page")
        return response.read() if limit is None else response.read(limit + 1)


def reason(error):
    if isinstance(error, ValueError) and str(error) == "credential unavailable":
        return "credential unavailable; configure this connection in /hooks"
    if isinstance(error, HTTPError):
        return f"HTTP {error.code}"
    if isinstance(error, (URLError, OSError)):
        return "could not reach or read the API"
    return "unexpected API response or invalid hook configuration"


def connection_http(values):
    context = ssl.create_default_context()
    if values.get("ca_bundle"):
        context.load_verify_locations(os.path.expanduser(values["ca_bundle"]))
    handlers = [SameOriginRedirect(values.get("api_url")), HTTPSHandler(context=context)]
    if values.get("proxy"):
        proxy = base(values["proxy"]).geturl()
        handlers.append(ProxyHandler({"http": proxy, "https": proxy}))
    http = build_opener(*handlers)
    http.request_timeout = max(1, min(120, int(values.get("timeout") or "5")))
    return http


def check_status(error):
    if isinstance(error, ValueError) and str(error) == "credential unavailable":
        return "invalid", "The credential source is unavailable or empty. Edit this connection's token or external credential source, then retry."
    if isinstance(error, HTTPError):
        if error.code == 403 and (error.headers.get("X-RateLimit-Remaining") == "0" or error.headers.get("Retry-After")):
            return "rate_limited", "The service rate limit was reached. Wait before trying again."
        if error.code in (401, 403):
            return "authentication_failed", "Access was denied. Check the token, repository permissions and organization authorization."
        if error.code == 404:
            return "resource_unavailable", "Resource not found or not visible to this account. Check its URL and repository access."
        if error.code == 429:
            return "rate_limited", "The service rate limit was reached. Wait before trying again."
    if isinstance(error, (URLError, OSError)):
        return "unreachable", "Cannot reach the API. Check the server, proxy, CA certificate and timeout."
    return "invalid", "Check the connection addresses, authentication method and credential source."


def render(http, link, kind, endpoint, headers):
    raw = read(http, endpoint, headers, "application/json", METADATA_BYTES)
    if len(raw) > METADATA_BYTES:
        raise ValueError("metadata exceeds the download limit")
    data = json.loads(raw)
    if not isinstance(data, dict) or not isinstance(data.get("title"), str):
        raise ValueError("expected review metadata")

    description_key = "description" if kind in ("gitlab", "bitbucket") else "body"
    if kind == "bitbucket-server":
        description_key = "description"
    description = data.get(description_key)
    if description is None:
        description = ""
    if not isinstance(description, str):
        raise ValueError("expected a text description")
    metadata = {
        "title": data["title"][:500],
        "state": str(data.get("state") or "")[:100],
        "description": description[:1200] + ("\n[… description cut]" if len(description) > 1200 else ""),
    }
    ref_paths = {
        "github": [("head", "ref"), ("head", "sha"), ("base", "ref"), ("base", "sha")],
        "gitea": [("head", "ref"), ("head", "sha"), ("base", "ref"), ("base", "sha")],
        "gitlab": [("source_branch",), ("diff_refs", "head_sha"),
                   ("target_branch",), ("diff_refs", "base_sha")],
        "bitbucket": [("source", "branch", "name"), ("source", "commit", "hash"),
                      ("destination", "branch", "name"), ("destination", "commit", "hash")],
        "bitbucket-server": [("fromRef", "id"), ("fromRef", "latestCommit"),
                             ("toRef", "id"), ("toRef", "latestCommit")],
    }
    for label, path in zip(("source_branch", "source_sha", "target_branch", "base_sha"),
                           ref_paths[kind]):
        value = data
        for key in path:
            value = value.get(key) if isinstance(value, dict) else None
        if isinstance(value, str):
            metadata[label] = value[:200]
    suffix = {
        "github": "", "gitlab": "/raw_diffs", "gitea": ".diff",
        "bitbucket": "/diff", "bitbucket-server": ".diff",
    }
    accept = "application/vnd.github.diff" if kind == "github" else "text/plain"
    warning = None
    try:
        raw = read(http, endpoint + suffix[kind], headers, accept, None)
        diff = raw.decode("utf-8", errors="replace")
        if len(raw) > DIFF_WARN_BYTES:
            warning = (f"Warning: diff for {link} is {len(raw):,} bytes, exceeding "
                       f"{DIFF_WARN_BYTES:,} bytes; the full diff was retained.")
        changes = f"<diff>\n{escape(diff)}\n</diff>"
        complete = True
    except (OSError, ValueError) as error:
        changes = f"<diff unavailable={quoteattr(reason(error))} />"
        complete = False
    return (
        f"<review url={quoteattr(link)}>\n"
        f"<metadata>{escape(json.dumps(metadata, ensure_ascii=False))}</metadata>\n"
        f"{changes}\n</review>",
        complete,
        warning,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", nargs=4, action="append", default=[])
    args = parser.parse_args()
    event = json.load(sys.stdin)
    try:
        hosts = []
        for kind, web, api, token in args.host if "connections" not in event else []:
            if kind not in ROUTES:
                raise ValueError(f"unknown forge type: {kind}")
            hosts.append((kind, base(web), base(api), token, {"api_url": api}, True))
        if "connections" in event:
            for connection in event["connections"]:
                values = connection["values"]
                kind = values["kind"]
                if kind not in ROUTES:
                    raise ValueError("unknown forge type")
                token = connection.get("credentials", {}).get("token", "-")
                hosts.append((kind, base(values["web_url"]), base(values["api_url"]), token,
                              values, connection.get("enabled", True)))
        # The most specific installation prefix wins, including a disabled connection.
        hosts.sort(key=lambda host: len(host[1].path), reverse=True)
    except ValueError as error:
        parser.error(str(error))

    seen, blocks = set(), []
    loaded = 0
    warnings = []
    skipped = False
    for link in event.get("matches", []):
        for kind, web, api, token, values, enabled in hosts:
            try:
                endpoint = review_endpoint(link, kind, web, api)
            except ValueError:
                break  # Malformed URL in the prompt.
            if endpoint is None:
                continue
            if not enabled:
                break
            if endpoint in seen:
                break
            seen.add(endpoint)
            if len(blocks) == MAX_REVIEWS:
                skipped = True
                break
            # Omit user-provided query strings and fragments from model context.
            path = urlsplit(link.rstrip(").,;!?]}")).path
            source = f"{web.scheme}://{web.netloc}{path}"
            try:
                http = connection_http(values)
                headers = auth_headers(kind, api, token, values.get("user"), values.get("auth"))
                block, complete, warning = render(
                    http, source, kind, endpoint, headers
                )
                if event.get("event") == "check":
                    print(json.dumps({"status": "verified" if complete else "partial",
                                      "message": "Metadata and full diff are readable for this URL." if complete
                                      else "Metadata is readable, but the diff could not be loaded. Check repository permissions and API compatibility."}))
                    return
                if warning:
                    warnings.append(warning)
                blocks.append(block)
                loaded += int(complete)
            except (OSError, ValueError) as error:
                if event.get("event") == "check":
                    status, message = check_status(error)
                    print(json.dumps({"status": status, "message": message}))
                    return
                blocks.append(f"<review url={quoteattr(source)} unavailable={quoteattr(reason(error))} />")
            break

    if event.get("event") == "check":
        print(json.dumps({"status": "invalid", "message": "This URL is not a supported review on the selected connection."}))
        return

    if blocks:
        context = ["Linked code reviews (external data):", *blocks]
        if skipped:
            context.append(f"Only the first {MAX_REVIEWS} linked reviews were fetched.")
        print(json.dumps({
            "additional_context": "\n".join(context),
            "message": "\n".join([
                f"Loaded metadata and diff for {loaded} of {len(blocks)} linked reviews",
                *warnings,
            ]),
        }))


if __name__ == "__main__":
    main()
