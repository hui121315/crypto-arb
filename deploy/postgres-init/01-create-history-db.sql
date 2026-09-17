SELECT 'CREATE DATABASE cryptoarb_history OWNER app'
WHERE NOT EXISTS (
    SELECT 1 FROM pg_database WHERE datname = 'cryptoarb_history'
)\gexec
