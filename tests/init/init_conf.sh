#!/bin/bash

cat <<EOF > /var/lib/postgresql/data/pg_hba.conf
local    all all     trust
host     all all all md5
hostssl  all all all cert
hostssl  all all all md5
EOF

mkfir -p /tmp/certs
chmod 600 /tmp/certs

cp /certs/{ca.pem,server.crt,server.key} /tmp/certs/
chmod 400 /tmp/certs/{ca.pem,server.crt,server.key}
