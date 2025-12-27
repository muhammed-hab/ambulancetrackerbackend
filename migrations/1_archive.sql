
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