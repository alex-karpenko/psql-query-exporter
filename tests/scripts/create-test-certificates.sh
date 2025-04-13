#!/bin/sh

# 1 - files prefix
#     may include path to destination folder
#     or may be used to generate multiple bundles at the same location

prefix="${1}"
basedir=$(dirname ${0})

openssl req -nodes -x509 -days 3650 -sha256 -batch -subj "/CN=Test RSA root CA" \
            -newkey rsa:4096 -keyout ${prefix}ca.key -out ${prefix}ca.crt

openssl req -nodes -sha256 -batch -subj "/CN=Test RSA intermediate CA" \
            -newkey rsa:3072 -keyout ${prefix}inter.key -out ${prefix}inter.req

openssl req -nodes -sha256 -batch -subj "/CN=test-server.com" \
            -newkey rsa:2048 -keyout ${prefix}end.key -out ${prefix}end.req

openssl rsa -in ${prefix}end.key -out ${prefix}test-server.key

openssl x509 -req -sha256 -days 3650 -set_serial 123 -extensions v3_inter -extfile ${basedir}/openssl.cnf \
             -CA ${prefix}ca.crt -CAkey ${prefix}ca.key -in ${prefix}inter.req -out ${prefix}inter.crt

openssl x509 -req -sha256 -days 2000 -set_serial 456 -extensions v3_end -extfile ${basedir}/openssl.cnf \
             -CA ${prefix}inter.crt -CAkey ${prefix}inter.key -in ${prefix}end.req -out ${prefix}end.crt

cat ${prefix}end.crt ${prefix}inter.crt > ${prefix}test-server.pem
cat ${prefix}inter.crt ${prefix}ca.crt > ${prefix}ca.pem
rm ${prefix}*.req ${prefix}ca.key ${prefix}inter.key ${prefix}end.key

mkdir tests/tls
cp ${prefix}ca.* tests/tls/
