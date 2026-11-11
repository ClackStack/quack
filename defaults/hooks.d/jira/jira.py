#!/usr/bin/env python3
"""A quack `user_prompt` hook: add the Jira issues a prompt links to.

quack runs this when the prompt matches the hook's `match.prompt` and writes the event
to stdin as JSON (`prompt`, and `matches`: the links it found). For each link to the
Jira at JIRA_URL, this fetches the issue over the REST API (Server, Data Center or
Cloud) and prints it as XML for the model, as `additional_context`.

Settings come from the environment, which the hook file sets with `env`:

  JIRA_URL           the Jira base URL, e.g. https://jira.example.com (required)
  JIRA_TOKEN         a personal access token (Server/Data Center), or an API token
                     (Cloud, together with JIRA_USER) (required)
  JIRA_USER          Cloud only: the account email; switches to basic authentication
  JIRA_MAX_ISSUES    issues fetched per prompt (default 5)
  JIRA_MAX_COMMENTS  newest comments kept per issue (default 5)
  JIRA_MAX_CHARS     longest description or comment kept, in characters (default 4000)
  JIRA_CA_BUNDLE     a PEM file of CA certificates (e.g. a corporate intermediate) added
                     to the system trust store, never replacing it. Pass the server's
                     intermediate here when its issuer root is already trusted by the system.
  JIRA_TIMEOUT       seconds per request (default 8)

Only the Python standard library is used. Exit status 1 with a message on stderr means
the hook is misconfigured; quack shows it as a warning and sends the prompt anyway.
"""

import base64
import json
import os
import re
import ssl
import sys
import urllib.error
import urllib.parse
import urllib.request
from xml.sax.saxutils import escape, quoteattr

FIELDS = ",".join(
    [
        "summary",
        "issuetype",
        "status",
        "priority",
        "resolution",
        "assignee",
        "reporter",
        "created",
        "updated",
        "labels",
        "components",
        "fixVersions",
        "description",
        "comment",
        "issuelinks",
        "subtasks",
        "parent",
    ]
)

KEY = re.compile(r"/browse/([A-Z][A-Z0-9_]+-\d+)")


def setting(name, default=None):
    value = os.environ.get(name, "").strip()
    return value or default


def number(name, default):
    try:
        return max(0, int(setting(name, str(default))))
    except ValueError:
        return default


def fail(message):
    print(message, file=sys.stderr)
    sys.exit(1)


def issue_keys(event, base):
    """Issue keys from links to `base`, in prompt order, without repeats."""
    links = event.get("matches") or re.findall(r"https?://\S+", event.get("prompt", ""))
    server = urllib.parse.urlsplit(base)
    keys = []
    for link in links:
        target = urllib.parse.urlsplit(link)
        if (target.scheme, target.netloc.lower()) != (server.scheme, server.netloc.lower()) or not target.path.startswith(server.path.rstrip("/") + "/"):
            continue
        m = KEY.search(link)
        if m and m.group(1) not in keys:
            keys.append(m.group(1))
    return keys


class SameOriginRedirect(urllib.request.HTTPRedirectHandler):
    def __init__(self, base):
        super().__init__()
        self.prefix = urllib.parse.urlsplit(base).path.rstrip("/")

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        old, new = urllib.parse.urlsplit(req.full_url), urllib.parse.urlsplit(newurl)
        old_port = old.port or (443 if old.scheme == "https" else 80)
        new_port = new.port or (443 if new.scheme == "https" else 80)
        if (old.scheme, old.hostname, old_port) != (new.scheme, new.hostname, new_port) or new.username:
            raise ValueError("API redirected to a different origin")
        if not new.path.startswith(self.prefix + "/"):
            raise ValueError("API redirected outside its configured prefix")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def opener(api=None):
    # Trust the system root store (on a corporate workstation this includes any
    # TLS-intercepting root enrolled there, e.g. GlobalSign Root CA - R3), then load
    # JIRA_CA_BUNDLE *on top of* it, never instead of it. Passing `cafile=` to
    # create_default_context() replaces the system store, so an intermediate such
    # as GlobalSign RSA OV SSL CA 2018 ends up trusted on its own and can no longer
    # reach its issuer root -> "unable to get issuer certificate".
    context = ssl.create_default_context()
    bundle = setting("JIRA_CA_BUNDLE")
    if bundle:
        path = os.path.expanduser(bundle)
        if os.path.exists(path):
            context.load_verify_locations(path)
        else:
            fail(f"JIRA_CA_BUNDLE is {path}, which does not exist; remove it or fix the path")
    # urllib honours HTTPS_PROXY and NO_PROXY, which quack passes through.
    handlers = [SameOriginRedirect(api or setting("JIRA_URL", "")), urllib.request.HTTPSHandler(context=context)]
    if setting("JIRA_PROXY"):
        handlers.append(urllib.request.ProxyHandler({"http": setting("JIRA_PROXY"), "https": setting("JIRA_PROXY")}))
    return urllib.request.build_opener(*handlers)


def authorization():
    token = setting("JIRA_TOKEN") or fail(
        "JIRA_TOKEN is not set; set it in the hook file's [env] (see hooks.d/jira.toml)"
    )
    user = setting("JIRA_USER")
    if user:
        pair = base64.b64encode(f"{user}:{token}".encode()).decode()
        return f"Basic {pair}"
    return f"Bearer {token}"


def fetch(http, base, auth, key):
    parsed = urllib.parse.urlsplit(base)
    if parsed.scheme != "https" and parsed.hostname not in ("localhost", "127.0.0.1", "::1"):
        raise ValueError("tokens require HTTPS")
    url = f"{base}/rest/api/2/issue/{urllib.parse.quote(key)}?fields={FIELDS}"
    request = urllib.request.Request(
        url, headers={"Authorization": auth, "Accept": "application/json"}
    )
    with http.open(request, timeout=number("JIRA_TIMEOUT", 8)) as response:
        return json.load(response)


def text(value, limit):
    if value and not isinstance(value, str):
        value = json.dumps(value, ensure_ascii=False)
    value = (value or "").strip()
    if len(value) > limit:
        value = value[:limit].rstrip() + f"\n[… cut at {limit} characters]"
    return escape(value)


def name(person):
    return (person or {}).get("displayName") or ""


def names(items):
    return ", ".join(i.get("name", "") for i in items or [] if i.get("name"))


def render(base, key, issue):
    f = issue.get("fields") or {}
    limit = number("JIRA_MAX_CHARS", 4000)
    out = [f"<jira_issue key={quoteattr(key)} url={quoteattr(f'{base}/browse/{key}')}>"]

    def tag(name_, value):
        if value:
            out.append(f"  <{name_}>{escape(str(value))}</{name_}>")

    tag("summary", f.get("summary"))
    tag("type", (f.get("issuetype") or {}).get("name"))
    tag("status", (f.get("status") or {}).get("name"))
    tag("priority", (f.get("priority") or {}).get("name"))
    tag("resolution", (f.get("resolution") or {}).get("name"))
    tag("assignee", name(f.get("assignee")))
    tag("reporter", name(f.get("reporter")))
    tag("created", f.get("created"))
    tag("updated", f.get("updated"))
    tag("labels", ", ".join(f.get("labels") or []))
    tag("components", names(f.get("components")))
    tag("fix_versions", names(f.get("fixVersions")))
    parent = f.get("parent")
    if parent:
        summary = (parent.get("fields") or {}).get("summary", "")
        out.append(f"  <parent key={quoteattr(parent.get('key', ''))}>{escape(summary)}</parent>")
    if f.get("description"):
        out.append(f"  <description>\n{text(f.get('description'), limit)}\n  </description>")

    links = []
    for link in f.get("issuelinks") or []:
        kind = link.get("type") or {}
        if "outwardIssue" in link:
            other, relation = link["outwardIssue"], kind.get("outward", "relates to")
        elif "inwardIssue" in link:
            other, relation = link["inwardIssue"], kind.get("inward", "relates to")
        else:
            continue
        of = other.get("fields") or {}
        status = (of.get("status") or {}).get("name", "")
        links.append(
            f"    <link relation={quoteattr(relation)} key={quoteattr(other.get('key', ''))}"
            f" status={quoteattr(status)}>{escape(of.get('summary', ''))}</link>"
        )
    for sub in f.get("subtasks") or []:
        sf = sub.get("fields") or {}
        status = (sf.get("status") or {}).get("name", "")
        links.append(
            f"    <link relation=\"subtask\" key={quoteattr(sub.get('key', ''))}"
            f" status={quoteattr(status)}>{escape(sf.get('summary', ''))}</link>"
        )
    if links:
        out += ["  <links>", *links, "  </links>"]

    comments = (f.get("comment") or {}).get("comments") or []
    keep = number("JIRA_MAX_COMMENTS", 5)
    if comments and keep:
        shown = comments[-keep:]
        out.append(f'  <comments shown="{len(shown)}" total="{len(comments)}">')
        for c in shown:
            out.append(
                f"    <comment author={quoteattr(name(c.get('author')))}"
                f" created={quoteattr(c.get('created', ''))}>\n"
                f"{text(c.get('body'), limit)}\n    </comment>"
            )
        out.append("  </comments>")
    out.append("</jira_issue>")
    return "\n".join(out)


def unavailable(key, why):
    return f"<jira_issue key={quoteattr(key)} unavailable={quoteattr(why)} />"


def main():
    event = json.load(sys.stdin)
    if "connections" in event:
        structured_main(event)
        return
    base = (setting("JIRA_URL") or fail(
        "JIRA_URL is not set; set it in the hook file's [env] (see hooks.d/jira.toml)"
    )).rstrip("/")
    keys = issue_keys(event, base)[: number("JIRA_MAX_ISSUES", 5)]
    if not keys:
        return  # links to another Jira: nothing to add
    auth = authorization()
    http = opener()
    blocks, fetched = [], []
    for key in keys:
        try:
            blocks.append(render(base, key, fetch(http, base, auth, key)))
            fetched.append(key)
        except urllib.error.HTTPError as e:
            if e.code == 401:
                fail(f"Jira at {base} rejected the token (HTTP 401); check JIRA_TOKEN")
            why = "not found, or no permission to see it" if e.code == 404 else f"HTTP {e.code}"
            blocks.append(unavailable(key, why))
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            reason = getattr(e, "reason", e)
            fail(f"cannot reach Jira at {base}: {reason}")
        except ValueError:
            blocks.append(unavailable(key, "Jira answered with something that is not JSON"))
    missing = [k for k in keys if k not in fetched]
    message = f"fetched {', '.join(fetched)}" if fetched else "no issues fetched"
    if missing:
        message += f"; could not fetch {', '.join(missing)}"
    print(json.dumps({
        "additional_context": "Jira issues linked in this message:\n" + "\n".join(blocks),
        "message": message,
    }))


def structured_main(event):
    # An explicit list, even an empty one, replaces the old environment setup.
    connections = sorted(event["connections"], key=lambda c: len(c["values"].get("web_url", "")), reverse=True)
    blocks, fetched, missing, seen = [], [], [], set()
    for link in event.get("matches", []):
        for connection in connections:
            values = connection["values"]
            web = values.get("web_url", "").rstrip("/")
            keys = issue_keys({"matches": [link]}, web)
            if not keys:
                continue
            if not connection.get("enabled", True):
                break
            key = keys[0]
            identity = (web, key)
            if identity in seen:
                break
            seen.add(identity)
            if len(blocks) >= max(1, int(values.get("max_issues") or "5")):
                break
            try:
                api = values.get("api_url", web).rstrip("/")
                parsed = urllib.parse.urlsplit(api)
                if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.query or parsed.fragment:
                    raise ValueError("invalid API address")
                if parsed.scheme != "https" and parsed.hostname not in ("localhost", "127.0.0.1", "::1"):
                    raise ValueError("tokens require HTTPS")
                variable = connection.get("credentials", {}).get("token", "")
                token = os.environ.get(variable, "")
                if not token:
                    raise ValueError("credential unavailable")
                # The legacy helpers use these names only within this short-lived process.
                settings = {"JIRA_TOKEN": token, "JIRA_USER": values.get("user", "") if values.get("auth") == "basic" else "",
                            "JIRA_CA_BUNDLE": values.get("ca_bundle", ""), "JIRA_PROXY": values.get("proxy", ""),
                            "JIRA_TIMEOUT": values.get("timeout", "5"), "JIRA_MAX_COMMENTS": values.get("max_comments", "5"),
                            "JIRA_MAX_CHARS": values.get("max_chars", "4000")}
                os.environ.update(settings)
                issue = fetch(opener(api), api, authorization(), key)
                if not isinstance(issue, dict) or not isinstance(issue.get("fields"), dict):
                    raise ValueError("expected issue metadata")
                block = render(web, key, issue)
                if event.get("event") == "check":
                    print(json.dumps({"status": "verified", "message": "Issue metadata is readable for this URL."}))
                    return
                blocks.append(block)
                fetched.append(key)
            except (OSError, ValueError, SystemExit) as error:
                status, message = "invalid", "Check the connection addresses, authentication method and credential source."
                if isinstance(error, urllib.error.HTTPError):
                    if error.code in (401, 403):
                        status, message = "authentication_failed", "Access denied. Check the token, account email and issue permissions."
                    elif error.code == 404:
                        status, message = "resource_unavailable", "Issue not found or not visible to this account."
                    elif error.code == 429:
                        status, message = "rate_limited", "Rate limit reached. Wait before trying again."
                elif isinstance(error, OSError):
                    status, message = "unreachable", "Cannot reach Jira. Check the server, proxy, CA certificate and timeout."
                if event.get("event") == "check":
                    print(json.dumps({"status": status, "message": message}))
                    return
                blocks.append(unavailable(key, message))
                missing.append(key)
            break
    if event.get("event") == "check":
        print(json.dumps({"status": "invalid", "message": "This URL is not a Jira issue on the selected connection."}))
    elif blocks:
        message = f"fetched {', '.join(fetched)}" if fetched else "no issues fetched"
        if missing:
            message += f"; could not fetch {', '.join(missing)}"
        print(json.dumps({"additional_context": "Jira issues linked in this message:\n" + "\n".join(blocks), "message": message}))


if __name__ == "__main__":
    main()
