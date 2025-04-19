#!/bin/bash

cat <<EOF > /var/lib/postgresql/data/pg_hba.conf
local    all all     trust
host     all all all md5
hostssl  all all all cert
hostssl  all all all md5
EOF

chown postgres /certs/*
chmod 400 /certs/*
