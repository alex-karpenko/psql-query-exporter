#!/bin/sh

# 1 - files prefix
#     may include path to destination folder
#     or may be used to generate multiple bundles at the same location

prefix="tests/tls/"
basedir=$(dirname ${0})

mkdir -p ${prefix}

openssl req -nodes -x509 -days 3650 -sha256 -batch -subj "/CN=Test RSA root CA" \
            -newkey rsa:4096 -keyout ${prefix}ca.key -out ${prefix}ca.crt

openssl req -nodes -sha256 -batch -subj "/CN=localhost" \
            -newkey rsa:2048 -keyout ${prefix}server.key -out ${prefix}server.req

openssl req -nodes -sha256 -batch -subj "/CN=exporter" \
            -newkey rsa:2048 -keyout ${prefix}client.key -out ${prefix}client.req

openssl rsa -in ${prefix}server.key -out ${prefix}server.key
openssl rsa -in ${prefix}client.key -out ${prefix}client.key

openssl x509 -req -sha256 -days 3650 -set_serial 123 -extensions v3_end -extfile ${basedir}/openssl.cnf \
             -CA ${prefix}ca.crt -CAkey ${prefix}ca.key -in ${prefix}server.req -out ${prefix}server.crt

openssl x509 -req -sha256 -days 2000 -set_serial 456 \
             -CA ${prefix}ca.crt -CAkey ${prefix}ca.key -in ${prefix}client.req -out ${prefix}client.crt

rm ${prefix}*.req ${prefix}ca.key
mv ${prefix}ca.crt ${prefix}ca.pem
