#!/usr/bin/env python3
"""Public certificate and private TLS key for one newly owned loopback server.

No existing trust store is modified. Tests explicitly trust the generated
certificate for the owned origin and retain normal upstream verification.
"""
import base64
from datetime import datetime, timedelta, timezone
import hashlib
import ipaddress
import os
import ssl

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID


def server_certificate(directory, label, dns_names=()):
    key = ec.generate_private_key(ec.SECP256R1())
    root_key = ec.generate_private_key(ec.SECP256R1())
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "M119 owned loopback")])
    root_name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "M119 owned local test authority")])
    now = datetime.now(timezone.utc)
    root = (x509.CertificateBuilder().subject_name(root_name).issuer_name(root_name).public_key(root_key.public_key())
        .serial_number(x509.random_serial_number()).not_valid_before(now - timedelta(minutes=5))
        .not_valid_after(now + timedelta(days=1)).add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(root_key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(root_key.public_key()), critical=False)
        .add_extension(x509.KeyUsage(digital_signature=True, content_commitment=False, key_encipherment=False,
            data_encipherment=False, key_agreement=False, key_cert_sign=True, crl_sign=True, encipher_only=None, decipher_only=None), critical=True)
        .sign(root_key, hashes.SHA256()))
    certificate = (x509.CertificateBuilder().subject_name(name).issuer_name(root_name).public_key(key.public_key())
        .serial_number(x509.random_serial_number()).not_valid_before(now - timedelta(minutes=5))
        .not_valid_after(now + timedelta(days=1)).add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(x509.SubjectAlternativeName([x509.IPAddress(ipaddress.ip_address("127.0.0.1")),
            *(x509.DNSName(name) for name in dns_names)]), critical=False)
        .add_extension(x509.SubjectKeyIdentifier.from_public_key(key.public_key()), critical=False)
        .add_extension(x509.AuthorityKeyIdentifier.from_issuer_public_key(root_key.public_key()), critical=False)
        .add_extension(x509.KeyUsage(digital_signature=True, content_commitment=False, key_encipherment=False,
            data_encipherment=False, key_agreement=False, key_cert_sign=False, crl_sign=False, encipher_only=None, decipher_only=None), critical=True)
        .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), critical=False)
        .sign(root_key, hashes.SHA256()))
    key_path, cert_path = directory / (label + "-key.pem"), directory / (label + "-cert.pem")
    for path, content in [(key_path, key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption())),
                          (cert_path, certificate.public_bytes(serialization.Encoding.PEM) + root.public_bytes(serialization.Encoding.PEM))]:
        descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert_path, key_path)
    public_key = certificate.public_key().public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    pin = base64.b64encode(hashlib.sha256(public_key).digest()).decode()
    return context, cert_path, root.public_bytes(serialization.Encoding.PEM).decode(), pin
