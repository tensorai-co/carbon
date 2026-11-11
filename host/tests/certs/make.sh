#!/bin/sh
# Makes the certificates host's tests check, in DER: a test root, an intermediate, and leaves under it for a server
# (carbon.test) and a client; a self-signed server certificate; and a real chain, registry-1.docker.io's, as its server
# sends it. The tests check them all at one fixed time inside every certificate's dates (see the tests), so they don't
# go stale; running this again means moving that time.
set -e
cd "$(dirname "$0")"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cat > "$work/ext" <<'X'
[ca]
basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash
[server]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = DNS:carbon.test
[client]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = clientAuth
subjectAltName = DNS:carbon.test
X
key() { openssl ecparam -name prime256v1 -genkey -noout -out "$work/$1.key"; }
csr() { openssl req -new -key "$work/$1.key" -subj "/CN=$2" -out "$work/$1.csr"; }
sign() { openssl x509 -req -in "$work/$1.csr" -CA "$work/$2.pem" -CAkey "$work/$2.key" -CAcreateserial -sha256 -days "$4" -extfile "$work/ext" -extensions "$3" -out "$work/$1.pem"; }
self() { openssl x509 -req -in "$work/$1.csr" -signkey "$work/$1.key" -sha256 -days "$3" -extfile "$work/ext" -extensions "$2" -out "$work/$1.pem"; }

# CAs for a century; leaves for 397 days, as TLS servers' are (macOS refuses longer ones).
key root; csr root "Carbon Test Root"; self root ca 36500
key intermediate; csr intermediate "Carbon Test Intermediate"; sign intermediate root ca 36500
key server; csr server "carbon.test"; sign server intermediate server 397
key client; csr client "carbon.test client"; sign client intermediate client 397
key selfsigned; csr selfsigned "carbon.test"; self selfsigned server 397
for name in root intermediate server client selfsigned; do
    openssl x509 -in "$work/$name.pem" -outform DER -out "$name.der"
done

echo | openssl s_client -connect registry-1.docker.io:443 -servername registry-1.docker.io -showcerts 2>/dev/null |
    awk -v dir="$work" '/BEGIN CERT/ { n++; f = dir "/docker-" n-1 ".pem" } f { print > f } /END CERT/ { f = "" }'
for pem in "$work"/docker-*.pem; do
    openssl x509 -in "$pem" -outform DER -out "$(basename "$pem" .pem).der"
done
