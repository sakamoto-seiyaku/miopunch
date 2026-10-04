#!/usr/bin/env python3
"""Create an owned test CA and import it into a private NSS database only."""
import ctypes
import ctypes.util
import ipaddress
from pathlib import Path
import re
import ssl
import subprocess


def certificates(directory: Path, *, dns_names=('localhost',), ip_addresses=('127.0.0.1',)):
    """Ephemeral keys stay in the temporary fixture and are never evidence."""
    if not dns_names or len(dns_names) + len(ip_addresses) > 16 or any(
        not re.fullmatch(r'[A-Za-z0-9.-]{1,253}', name) for name in dns_names
    ):
        raise ValueError('invalid bounded owned certificate names')
    addresses = [str(ipaddress.ip_address(address)) for address in ip_addresses]
    names = ','.join([f'DNS:{name}' for name in dns_names] + [f'IP:{address}' for address in addresses])
    def openssl(*args):
        subprocess.run(['rtk', 'proxy', 'openssl', *args], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
    ca = directory / 'ca.pem'
    key = directory / 'ca.key'
    leaf = directory / 'leaf.pem'
    leaf_key = directory / 'leaf.key'
    request = directory / 'leaf.csr'
    extensions = directory / 'leaf.ext'
    openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
            '-subj', '/CN=miopunch owned MQTT test CA',
            '-addext', 'basicConstraints=critical,CA:TRUE',
            '-addext', 'keyUsage=critical,keyCertSign,cRLSign',
            '-keyout', str(key), '-out', str(ca))
    openssl('req', '-newkey', 'rsa:2048', '-nodes', '-subj', f'/CN={dns_names[0]}',
            '-keyout', str(leaf_key), '-out', str(request))
    extensions.write_text('basicConstraints=critical,CA:FALSE\n'
                          'keyUsage=critical,digitalSignature,keyEncipherment\n'
                          'extendedKeyUsage=serverAuth\n'
                          f'subjectAltName={names}\n')
    openssl('x509', '-req', '-in', str(request), '-CA', str(ca), '-CAkey', str(key),
            '-CAcreateserial', '-days', '1', '-extfile', str(extensions), '-out', str(leaf))
    for name in dns_names:
        openssl('verify', '-CAfile', str(ca), '-verify_hostname', name, str(leaf))
    return ca, leaf, leaf_key


def private_nss(directory: Path, ca: Path):
    """Use installed NSS APIs; do not install tools or touch the user's DB.

    Chromium's NSS directory is mapped to this DB inside its private bwrap
    mount namespace. The normal verifier still checks chain, time and hostname.
    API contracts: nss-dev/nss lib/certdb/cert.h and lib/util/seccomon.h.
    """
    directory.mkdir()
    nss = ctypes.CDLL(ctypes.util.find_library('nss3'))
    pointer = ctypes.c_void_p

    class SECItem(ctypes.Structure):
        _fields_ = [('type', ctypes.c_uint), ('data', pointer), ('len', ctypes.c_uint)]

    class Trust(ctypes.Structure):
        _fields_ = [('ssl', ctypes.c_uint), ('email', ctypes.c_uint), ('object', ctypes.c_uint)]

    signatures = {
        'NSS_InitReadWrite': ([ctypes.c_char_p], ctypes.c_int),
        'CERT_GetDefaultCertDB': ([], pointer),
        'CERT_NewTempCertificate': ([pointer, ctypes.POINTER(SECItem), ctypes.c_char_p, ctypes.c_int, ctypes.c_int], pointer),
        'CERT_DecodeTrustString': ([ctypes.POINTER(Trust), ctypes.c_char_p], ctypes.c_int),
        '__CERT_AddTempCertToPerm': ([pointer, ctypes.c_char_p, ctypes.POINTER(Trust)], ctypes.c_int),
        'CERT_DestroyCertificate': ([pointer], None),
        'PK11_GetInternalKeySlot': ([], pointer),
        'PK11_NeedUserInit': ([pointer], ctypes.c_int),
        'PK11_InitPin': ([pointer, ctypes.c_char_p, ctypes.c_char_p], ctypes.c_int),
        'PK11_FreeSlot': ([pointer], None),
        'PORT_GetError': ([], ctypes.c_int),
        'NSS_Shutdown': ([], ctypes.c_int),
    }
    for name, (arguments, result) in signatures.items():
        function = getattr(nss, name)
        function.argtypes = arguments
        function.restype = result
    assert nss.NSS_InitReadWrite(f'sql:{directory}'.encode()) == 0, 'private NSS init failed'
    cert = None
    try:
        slot = nss.PK11_GetInternalKeySlot()
        assert slot, 'private NSS token missing'
        try:
            if nss.PK11_NeedUserInit(slot):
                assert nss.PK11_InitPin(slot, None, None) == 0, 'private NSS token initialization failed'
        finally:
            nss.PK11_FreeSlot(slot)
        der = ssl.PEM_cert_to_DER_cert(ca.read_text())
        data = ctypes.create_string_buffer(der)
        item = SECItem(0, ctypes.cast(data, pointer), len(der))
        cert = nss.CERT_NewTempCertificate(nss.CERT_GetDefaultCertDB(), ctypes.byref(item), None, 0, 1)
        assert cert, 'private CA decode failed'
        trust = Trust()
        assert nss.CERT_DecodeTrustString(ctypes.byref(trust), b'C,,') == 0
        assert getattr(nss, '__CERT_AddTempCertToPerm')(cert, b'miopunch-owned-mqtt', ctypes.byref(trust)) == 0, f'private CA import failed: NSS {nss.PORT_GetError()}'
    finally:
        if cert:
            nss.CERT_DestroyCertificate(cert)
        assert nss.NSS_Shutdown() == 0, 'private NSS cleanup failed'
