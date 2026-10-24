"""Generate disposable database TLS fixtures; never read user credentials."""
from pathlib import Path
import os
import subprocess

root = Path(__file__).resolve().parent.parent / ".cache" / "mysql-fixture-tls"
root.mkdir(parents=True, exist_ok=True)
names = ["ca.pem", "ca-key.pem", "server.pem", "server-key.pem", "server.csr", "server.ext", "ca.srl"]
if any((root / name).is_symlink() for name in names):
    raise SystemExit("Refusing to replace symlinked fixture files")

def run(*args):
    subprocess.run(["openssl", *args], cwd=root, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

if not (root / "server.pem").exists() or subprocess.run(
    ["openssl", "x509", "-in", str(root / "server.pem"), "-checkend", "3600", "-noout"],
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
).returncode:
    run("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca-key.pem", "-out", "ca.pem", "-subj", "/CN=Joinery disposable database CA", "-days", "2")
    run("req", "-newkey", "rsa:2048", "-nodes", "-keyout", "server-key.pem", "-out", "server.csr", "-subj", "/CN=database.fixture.test")
    (root / "server.ext").write_text("subjectAltName=DNS:database.fixture.test,DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\nkeyUsage=digitalSignature,keyEncipherment\n")
    run("x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca-key.pem", "-CAcreateserial", "-out", "server.pem", "-days", "2", "-sha256", "-extfile", "server.ext")
    os.chmod(root / "ca-key.pem", 0o600)
    # Both official fixture images must be able to read the synthetic leaf key.
    os.chmod(root / "server-key.pem", 0o644)
print("Disposable MySQL/MariaDB TLS fixtures ready.")
