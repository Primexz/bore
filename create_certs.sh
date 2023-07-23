#!/bin/sh

mkdir certs

openssl req -x509 \
            -sha256 -days 356 \
            -nodes \
            -newkey rsa:2048 \
            -subj "/CN=BoreCA/C=DE/L=Hamburg" \
            -keyout certs/rootCA.key -out certs/rootCA.crt 

openssl genrsa -out certs/server.key 2048

cat > certs/csr.conf <<EOF
[ req ]
default_bits = 2048
prompt = no
default_md = sha256
req_extensions = req_ext
distinguished_name = dn

[ dn ]
C = DE
ST = Hamburg
L = Hamburg
O = a
OU = a
CN = localhost

[ req_ext ]
subjectAltName = @alt_names

[ alt_names ]
DNS.1 = localhost
EOF

openssl req -new -key certs/server.key -out certs/server.csr -config certs/csr.conf

cat > certs/cert.conf <<EOF
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
subjectAltName = @alt_names

[alt_names]
DNS.1 = localhost
EOF

openssl x509 -req \
    -in certs/server.csr \
    -CA certs/rootCA.crt -CAkey certs/rootCA.key \
    -out certs/server.crt \
    -days 365 \
    -sha256 -extfile certs/cert.conf

openssl pkcs12 -export -out certs/identity.pfx -inkey certs/server.key -in certs/server.crt -certfile certs/rootCA.crt -passout pass:1234

rm certs/server.csr certs/csr.conf certs/cert.conf