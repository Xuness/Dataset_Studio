"""Read the explicitly embedded Ideas resources; HTML remains the archived response."""

from html.parser import HTMLParser
import json
from urllib.parse import urlsplit


class InitialProps(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=False)
        self.active = False
        self.chunks = []
        self.matches = 0

    def handle_starttag(self, tag, attrs):
        if tag == "script" and dict(attrs).get("id") == "__PWS_INITIAL_PROPS__":
            self.active = True
            self.matches += 1

    def handle_data(self, data):
        if self.active:
            self.chunks.append(data)

    def handle_endtag(self, tag):
        if tag == "script":
            self.active = False


def document(body, url):
    parser = InitialProps()
    parser.feed(body.decode("utf-8"))
    if parser.matches != 1:
        raise ValueError("Ideas initial resources are absent or ambiguous")
    resources = json.loads("".join(parser.chunks))["initialReduxState"]["resources"]
    topic_id = urlsplit(url).path.strip("/").split("/")[-1]
    def select(name):
        found = []
        for key, value in resources[name].items():
            options = dict(json.loads(key))
            if options.get("interest") == topic_id:
                found.append((options, value))
        if len(found) != 1:
            raise ValueError("Ideas resource identity is absent or ambiguous")
        return found[0]
    _, interest = select("InterestResource")
    options, feed = select("BestPinsFeedAltResource")
    if not isinstance(interest["data"], dict) or str(interest["data"].get("id")) != topic_id:
        raise ValueError("Ideas topic identity differs")
    bookmark = feed.get("nextBookmark")
    bookmarks = bookmark if isinstance(bookmark, list) else [bookmark] if isinstance(bookmark, str) else None
    return dict(resource_response=dict(status="success", data=feed["data"]), resource=dict(options={**options, "bookmarks": bookmarks})), interest["data"], options
