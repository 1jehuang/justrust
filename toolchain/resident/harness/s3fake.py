"""What remote.py puts between ra-jit and an S3 service: the service itself, started here, a
client that signs requests the way the service asks (so the harness can look at a bucket and
spoil what is in it without ra-jit), and a proxy that counts the requests and breaks them.

Nothing here is ra-jit's: the client is the standard library's, so that a mistake in how ra-jit
signs or addresses is not made twice."""
import datetime, hashlib, hmac, http.client, os, re, select, shutil, signal, socket, ssl, subprocess, threading, time
import urllib.parse
import xml.etree.ElementTree as ET

ACCESS, SECRET = "rajitharness", "rajitharness-not-a-secret"


def free_port(below=65536):
    """A port nothing listens on. SeaweedFS takes the port 10000 above each of its own too."""
    while True:
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        if port < below:
            return port


def wait_for(port, seconds=30):
    end = time.time() + seconds
    while time.time() < end:
        try:
            socket.create_connection(("127.0.0.1", port), timeout=0.2).close()
            return
        except OSError:
            time.sleep(0.05)
    raise SystemExit(f"nothing listens on {port} after {seconds}s")


# A service started with this is the head of a group or tree that `kill` ends whole.
OWN_GROUP = dict(creationflags=subprocess.CREATE_NEW_PROCESS_GROUP) if os.name == "nt" else dict(start_new_session=True)


class Server:
    """An S3 service on this machine, which can be killed and started again on the same data."""
    def __init__(self, kind, binary, top, certs=None):
        """`certs` is a directory with `public.crt` and `private.key` for a MinIO that speaks TLS."""
        self.kind, self.binary, self.dir, self.certs = kind, binary, os.path.join(top, kind), certs
        self.port, self.process = free_port(50000), None
        os.makedirs(self.dir, exist_ok=True)
        self.log = open(os.path.join(top, kind + ".log"), "ab")

    def start(self):
        if self.kind == "minio":
            env = {**os.environ, "MINIO_ROOT_USER": ACCESS, "MINIO_ROOT_PASSWORD": SECRET, "MINIO_DOMAIN": "localhost",
                   "HOME": self.dir}
            command = [self.binary, "server", self.dir, "--address", f"127.0.0.1:{self.port}", "--console-address",
                       f"127.0.0.1:{free_port()}", *(["--certs-dir", self.certs] if self.certs else [])]
        else:
            config = os.path.join(self.dir, "s3.json")
            open(config, "w").write('{"identities":[{"name":"ra-jit","credentials":[{"accessKey":"%s","secretKey":"%s"}],'
                                    '"actions":["Admin","Read","Write","List","Tagging"]}]}' % (ACCESS, SECRET))
            if not hasattr(self, "ports"):
                self.ports = [free_port(50000) for _ in range(3)]
            master, volume, filer = self.ports
            env = dict(os.environ)
            command = [self.binary, "server", f"-dir={self.dir}", "-ip=127.0.0.1", "-s3", f"-s3.port={self.port}",
                       f"-s3.config={config}", f"-master.port={master}", f"-volume.port={volume}", f"-filer.port={filer}",
                       "-volume.max=100"]
        self.process = subprocess.Popen(command, env=env, stdout=self.log, stderr=self.log, **OWN_GROUP)
        wait_for(self.port)

    def kill(self):
        if self.process:
            if os.name == "nt":
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(self.process.pid)], capture_output=True)
            else:
                os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait()
            self.process = None


class Bucket:
    """A bucket as the harness reads and writes it: signed requests, addressed by path."""
    def __init__(self, endpoint, bucket, prefix="", access=ACCESS, secret=SECRET, region="us-east-1", token=None, ca=None):
        """`ca` is the file of the certificate an `https` endpoint of the harness's own is trusted by."""
        self.ca = ca
        url = urllib.parse.urlsplit(endpoint)
        self.secure, self.host = url.scheme == "https", url.netloc
        self.bucket, self.prefix = bucket, prefix
        self.access, self.secret, self.region, self.token = access, secret, region, token

    def request(self, method, key="", query="", body=b""):
        path = "/" + self.bucket + ("/" + urllib.parse.quote(key) if key else "")
        now = datetime.datetime.now(datetime.timezone.utc)
        stamp, day = now.strftime("%Y%m%dT%H%M%SZ"), now.strftime("%Y%m%d")
        digest = hashlib.sha256(body).hexdigest()
        headers = {"host": self.host, "x-amz-content-sha256": digest, "x-amz-date": stamp}
        if self.token:
            headers["x-amz-security-token"] = self.token
        pairs = sorted(urllib.parse.parse_qsl(query, keep_blank_values=True))
        canonical_query = "&".join(f"{urllib.parse.quote(k, safe='')}={urllib.parse.quote(v, safe='')}" for k, v in pairs)
        signed = ";".join(sorted(headers))
        canonical = "\n".join([method, path, canonical_query, *(f"{k}:{headers[k]}" for k in sorted(headers)), "", signed, digest])
        scope = f"{day}/{self.region}/s3/aws4_request"
        to_sign = "\n".join(["AWS4-HMAC-SHA256", stamp, scope, hashlib.sha256(canonical.encode()).hexdigest()])
        key_ = ("AWS4" + self.secret).encode()
        for part in (day, self.region, "s3", "aws4_request"):
            key_ = hmac.new(key_, part.encode(), hashlib.sha256).digest()
        signature = hmac.new(key_, to_sign.encode(), hashlib.sha256).hexdigest()
        headers["authorization"] = f"AWS4-HMAC-SHA256 Credential={self.access}/{scope}, SignedHeaders={signed}, Signature={signature}"
        if self.secure:
            trusted = ssl.create_default_context(cafile=self.ca)
            connection = http.client.HTTPSConnection(self.host, timeout=60, context=trusted)
        else:
            connection = http.client.HTTPConnection(self.host, timeout=60)
        try:
            connection.request(method, path + ("?" + canonical_query if canonical_query else ""), body=body, headers=headers)
            response = connection.getresponse()
            return response.status, response.read()
        except OSError as error:
            return 0, str(error).encode()
        finally:
            connection.close()

    def must(self, method, key="", query="", body=b"", ok=(200, 204)):
        status, data = self.request(method, key, query, body)
        if status not in ok:
            raise SystemExit(f"{method} {key}?{query} answered {status}: {data[:300]!r}")
        return data

    def create(self):
        """Makes the bucket. A server that has only just started says 503 until it is ready."""
        for _ in range(100):
            status, data = self.request("PUT")
            if status != 503:
                break
            time.sleep(0.1)
        if status not in (200, 409):
            raise SystemExit(f"creating the bucket answered {status}: {data[:300]!r}")

    def get(self, name):
        status, data = self.request("GET", self.prefix + name)
        return data if status == 200 else None

    def put(self, name, body):
        self.must("PUT", self.prefix + name, body=body)

    def delete(self, name):
        self.must("DELETE", self.prefix + name)

    def _listed(self, query, tag):
        names, more = [], ""
        while True:
            root = ET.fromstring(self.must("GET", query=query + more))
            space = root.tag[:root.tag.index("}") + 1] if root.tag.startswith("{") else ""
            names += [(it.findtext(space + "Key"), it.findtext(space + "UploadId"), int(it.findtext(space + "Size") or 0))
                      for it in root.iter(space + tag)]
            token = root.findtext(space + "NextContinuationToken")
            if not token or "uploads" in query:
                return names
            more = "&continuation-token=" + urllib.parse.quote(token, safe="")

    def objects(self):
        """The names under the prefix, and the size of each."""
        listed = self._listed("list-type=2&prefix=" + urllib.parse.quote(self.prefix, safe=""), "Contents")
        return {key[len(self.prefix):]: size for key, _, size in listed}

    def uploads(self):
        """The multipart uploads under the prefix that were neither completed nor aborted."""
        listed = self._listed("uploads=&prefix=" + urllib.parse.quote(self.prefix, safe=""), "Upload")
        return [(key, upload) for key, upload, _ in listed]

    def empty(self):
        for name in self.objects():
            self.delete(name)
        for key, upload in self.uploads():
            self.must("DELETE", key, query="uploadId=" + urllib.parse.quote(upload, safe=""))


class Fault:
    """What the proxy does to the next `times` requests whose method and target match.

    `status`: answers that instead of asking the service. `reset`: closes without an answer.
    `cut`: closes after that many bytes of the answer's body. `stall`: `(bytes, seconds)`, stops
    for that long after that many bytes of the body. `cut_request`: closes after that many
    bytes of the request's body were read, before the service sees the request."""
    def __init__(self, method, target, times=1, status=None, reset=False, cut=None, stall=None, cut_request=None):
        self.method, self.target, self.times = method, re.compile(target), times
        self.status, self.reset, self.cut, self.stall, self.cut_request = status, reset, cut, stall, cut_request
        self.hits = 0


class Proxy:
    """Passes HTTP on to a service, one connection a request, and keeps what it saw: every
    request line with its `Range`, how many it held at once, the bytes of the bodies."""
    def __init__(self, upstream_port):
        self.upstream = upstream_port
        self.lock = threading.Lock()
        self.listener = socket.socket(socket.AF_INET6)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        self.listener.bind(("::", 0))
        self.listener.listen(128)
        self.port = self.listener.getsockname()[1]
        self.clear()
        threading.Thread(target=self._accept, daemon=True).start()

    def clear(self):
        with self.lock:
            self.requests, self.faults = [], []
            self.sent = self.received = self.flying = self.most_flying = 0
            # Bytes a second for the bodies, and seconds every request is held before it is passed on.
            self.rate, self.hold = None, 0
            # How many seconds earlier the service is made to say its objects were written, and
            # whether in its answers about one object too or only where it lists them.
            self.age, self.age_one = 0, True

    def fault(self, *args, **kwargs):
        fault = Fault(*args, **kwargs)
        with self.lock:
            self.faults.append(fault)
        return fault

    def seen(self, method, pattern=""):
        return [it for it in self.requests if it[0] == method and re.search(pattern, it[1])]

    def _accept(self):
        while True:
            client, _ = self.listener.accept()
            threading.Thread(target=self._serve, args=(client,), daemon=True).start()

    def _head(self, sock):
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = sock.recv(65536)
            if not chunk:
                return None, b""
            data += chunk
        head, rest = data.split(b"\r\n\r\n", 1)
        return head.decode("latin-1"), rest

    def _pace(self, count, start, total):
        if self.rate:
            ahead = total / self.rate - (time.time() - start)
            if ahead > 0:
                time.sleep(ahead)

    def _serve(self, client):
        try:
            self._request(client)
        except OSError:
            pass
        finally:
            client.close()

    def _request(self, client):
        head, rest = self._head(client)
        if head is None:
            return
        lines = head.split("\r\n")
        method, target, _ = lines[0].split(" ", 2)
        headers = {k.lower(): v.strip() for k, v in (line.split(":", 1) for line in lines[1:])}
        if "chunked" in headers.get("transfer-encoding", ""):
            raise SystemExit("the proxy does not pass on a chunked request")
        length = int(headers.get("content-length", 0))
        with self.lock:
            self.requests.append((method, target, headers.get("range")))
            fault = next((it for it in self.faults if it.times and it.method == method and it.target.search(target)), None)
            if fault:
                fault.times -= 1
                fault.hits += 1
            self.flying += bool(self.hold)
            self.most_flying = max(self.most_flying, self.flying)
        # A request counts as in flight while it is held: a client has its answer a moment
        # before the proxy is done with it, so counting to the end would count too many.
        if self.hold:
            time.sleep(self.hold)
            with self.lock:
                self.flying -= 1
        self._answer(client, lines, rest, length, fault)

    def _body(self, client, rest, length, stop_at=None):
        """The body of the request, or None when it was cut."""
        body, start = bytearray(rest), time.time()
        while len(body) < length:
            if stop_at is not None and len(body) >= stop_at:
                return None
            chunk = client.recv(min(1 << 16, length - len(body)))
            if not chunk:
                return None
            body += chunk
            self._pace(len(chunk), start, len(body))
        return bytes(body)

    def _answer(self, client, lines, rest, length, fault):
        if fault and fault.reset:
            return
        body = self._body(client, rest, length, fault.cut_request if fault else None)
        if body is None:
            return
        with self.lock:
            self.sent += len(body)
        if fault and fault.status:
            client.sendall(f"HTTP/1.1 {fault.status} Injected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".encode())
            return
        kept = [line for line in lines if not line.lower().startswith("connection:")]
        upstream = socket.create_connection(("127.0.0.1", self.upstream))
        try:
            upstream.sendall(("\r\n".join(kept) + "\r\nConnection: close\r\n\r\n").encode("latin-1") + body)
            self._relay(upstream, client, fault)
        finally:
            upstream.close()

    def _aged(self, head, rest, upstream):
        """An answer with the times objects were written moved back: the same number of bytes."""
        def earlier(found, form):
            then = datetime.datetime.strptime(found.group(2), form) - datetime.timedelta(seconds=self.age)
            return found.group(1) + then.strftime(form) + found.group(3)

        while chunk := upstream.recv(1 << 16):
            rest += chunk
        if self.age_one:
            head = re.sub(r"(?i)(last-modified: )([^\r]*)( GMT)", lambda it: earlier(it, "%a, %d %b %Y %H:%M:%S"), head)
        listed = re.sub(r"(<LastModified>)([^<.Z]*)((?:\.\d+)?Z</LastModified>)", lambda it: earlier(it, "%Y-%m-%dT%H:%M:%S"),
                        rest.decode("latin-1"))
        return head, listed.encode("latin-1")

    def _relay(self, upstream, client, fault):
        head, rest = self._head(upstream)
        if head is None:
            return
        if self.age:
            head, rest = self._aged(head, rest, upstream)
        lines = [line for line in head.split("\r\n") if not line.lower().startswith("connection:")]
        client.sendall(("\r\n".join(lines) + "\r\nConnection: close\r\n\r\n").encode("latin-1"))
        passed, start, stalled = 0, time.time(), False
        chunk = rest
        while True:
            if chunk:
                if fault and fault.cut is not None and passed + len(chunk) >= fault.cut:
                    client.sendall(chunk[:max(0, fault.cut - passed)])
                    client.shutdown(socket.SHUT_RDWR)
                    return
                client.sendall(chunk)
                passed += len(chunk)
                with self.lock:
                    self.received += len(chunk)
                self._pace(len(chunk), start, passed)
                if fault and fault.stall and not stalled and passed >= fault.stall[0]:
                    stalled = True
                    time.sleep(fault.stall[1])
            chunk = upstream.recv(1 << 16)
            if not chunk:
                return


def find(name, variable):
    """The binary of a service: what the variable names, one on PATH, or ~/s3tools/NAME."""
    exe = ".exe" if os.name == "nt" else ""
    for it in (os.environ.get(variable), shutil.which(name), os.path.expanduser(f"~/s3tools/{name}{exe}")):
        if it and os.path.exists(it):
            return it
    return None
