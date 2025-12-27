-- Migration: Create schema for ambulance tracking app

-- DROP TABLE IF EXISTS accounts CASCADE;
-- DROP TABLE IF EXISTS ambulance_locations CASCADE;
-- DROP TABLE IF EXISTS ambulances CASCADE;
-- DROP TABLE IF EXISTS eta_notifications CASCADE;
-- DROP TABLE IF EXISTS etas CASCADE;
-- DROP TABLE IF EXISTS live_tracking_sessions CASCADE;
-- DROP TABLE IF EXISTS phone_numbers CASCADE;
-- DROP TABLE IF EXISTS sessions CASCADE;
-- DROP TABLE IF EXISTS archive_ambulance_locations CASCADE;
-- DROP TABLE IF EXISTS archive_etas CASCADE;
-- DROP TYPE IF EXISTS account_role;

CREATE EXTENSION IF NOT EXISTS postgis;

-- ----------------------------------------
-- ENUM for account roles
-- ----------------------------------------
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'account_role') THEN
CREATE TYPE account_role AS ENUM ('admin','user','site_admin');
END IF;
END;
$$;

-- ----------------------------------------
-- Accounts
-- ----------------------------------------
CREATE TABLE accounts (
                          user_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                          username CHAR(16) NOT NULL UNIQUE,
                          password_hash BYTEA NOT NULL CHECK (octet_length(password_hash) = 32),
                          password_salt BYTEA NOT NULL CHECK (octet_length(password_salt) = 16),
                          role account_role NOT NULL DEFAULT 'user',
                          owner_id UUID REFERENCES accounts(user_id) ON DELETE CASCADE,
                          password_reset_needed BOOLEAN NOT NULL DEFAULT TRUE,
    -- making the decision to use geometry rather than geography because geo-zero in rust does not support geography
    -- because all the data should be relatively small, this should be fine for now and calculations will still be
    -- pretty accurate. however, if distances were to become large, distance calculations and similar would be off as
    -- geography is a true sphenoid where geometry assumes a plane
                          hospital GEOMETRY(POINT, 4326),
                          pref_eta INTERVAL NOT NULL DEFAULT INTERVAL '15 minutes'
);
CREATE INDEX idx_accounts_username ON accounts(username);
CREATE INDEX idx_accounts_owner_id ON accounts(owner_id);

-- ----------------------------------------
-- Sessions
-- ----------------------------------------
CREATE TABLE sessions (
                          session_id BYTEA PRIMARY KEY NOT NULL CHECK (octet_length(session_id) = 32),
                          user_id UUID NOT NULL REFERENCES accounts(user_id) ON DELETE CASCADE
);

-- ----------------------------------------
-- Phone numbers
-- ----------------------------------------
CREATE TABLE phone_numbers (
                               phone_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                               user_id UUID NOT NULL REFERENCES accounts(user_id) ON DELETE CASCADE,
                               phone CHAR(10) NOT NULL,
                               label VARCHAR(255)
);
CREATE INDEX idx_phone_numbers_user_id ON phone_numbers(user_id);

-- ----------------------------------------
-- Ambulances
-- ----------------------------------------
CREATE TABLE ambulances (
                            ambulance_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                            ambulance_name VARCHAR(255),
                            location GEOMETRY(POINT, 4326) NOT NULL,
                            last_update TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_ambulances_last_update ON ambulances (last_update);

-- ----------------------------------------
-- Live tracking sessions
-- ----------------------------------------
CREATE TABLE live_tracking_sessions (
                                        tracking_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                                        user_id UUID NOT NULL REFERENCES accounts(user_id) ON DELETE CASCADE,
                                        ambulance_id UUID NOT NULL REFERENCES ambulances(ambulance_id) ON DELETE CASCADE,
                                        user_description VARCHAR(1024),
                                        urgency CHAR(16),
                                        inserted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                                        arrived_at TIMESTAMPTZ,
                                        eta TIMESTAMPTZ,
                                        eta_last_calculated TIMESTAMPTZ,
                                        notify_self_at INTERVAL
);
CREATE UNIQUE INDEX idx_live_tracking_user ON live_tracking_sessions(user_id, ambulance_id);
CREATE INDEX idx_live_tracking_arrived ON live_tracking_sessions(arrived_at);
CREATE INDEX idx_live_tracking_eta_updates ON live_tracking_sessions(ambulance_id, eta_last_calculated);

-- ----------------------------------------
-- ETA notifications
-- ----------------------------------------
CREATE TABLE eta_notifications (
                                   tracking_id UUID NOT NULL REFERENCES live_tracking_sessions(tracking_id) ON DELETE CASCADE,
                                   notify_at_eta INTERVAL,
                                   fulfilled BOOLEAN NOT NULL DEFAULT FALSE,
                                   phone_id UUID REFERENCES phone_numbers(phone_id) ON DELETE CASCADE
);
CREATE INDEX idx_eta_notifications_tracking ON eta_notifications(tracking_id);
CREATE INDEX idx_eta_notifications_track_fulfilled_eta
    ON eta_notifications(tracking_id, fulfilled, notify_at_eta);


-- ----------------------------------------
-- ARCHIVE: Ambulance locations
-- ----------------------------------------
CREATE TABLE archive_ambulance_locations (
    ambulance_id UUID NOT NULL,
    ambulance_name VARCHAR(255),
    location GEOMETRY(POINT, 4326) NOT NULL,
    time TIMESTAMPTZ NOT NULL
);
CREATE TABLE archive_etas (
    ambulance_id UUID NOT NULL,
    current_location GEOMETRY(POINT, 4326) NOT NULL,
    destination GEOMETRY(POINT, 4326) NOT NULL,
    eta TIMESTAMPTZ NOT NULL,
    calculated_at TIMESTAMPTZ NOT NULL
);