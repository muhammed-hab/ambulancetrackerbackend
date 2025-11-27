# SQL Tables

Using PostGis extension for postgresql

### Accounts

| user_id              | username | password_hash | password_salt | role                        | owner_id                                                               | password_reset_needed | hospital             | pref_eta       |
|----------------------|----------|---------------|---------------|-----------------------------|------------------------------------------------------------------------|-----------------------|----------------------|----------------|
| uuid                 | char(16) | bytes(32)     | bytes(16)     | enum (admin/user/siteadmin) | uuid                                                                   | bool                  | WGS84 long/lat, NULL | time           |
| PK default random v4 | Unique   |               |               |                             | FK to Accounts user_id, owner_id must refer to role admin or siteadmin | default true          |                      | default 15 min |

- index on username
- index on owner_id

### Sessions

| session_id           | user_id        |
|----------------------|----------------|
| bytes(32)            | uuid           |
| PK default random v4 | FK to accounts |

### Phone numbers

| phone_id             | user_id        | phone    | label        |
|----------------------|----------------|----------|--------------|
| uuid                 | uuid           | char(10) | varchar(255) |
| PK default random v4 | FK to Accounts |          |              |

- index on user_id

### Ambulances

| ambulance_id         | ambulance_name | location       | last_update |
|----------------------|----------------|----------------|-------------|
| uuid                 | varchar(255)   | WGS84 long/lat | timestamp   |
| PK default random v4 |                |                |             |

- index on last_update

### Live tracking sessions

| tracking_id          | user_id     | ambulance id  | user_description | urgency  | inserted_at | arrived_at      | eta             | last_calculated | notify_self_at |
|----------------------|-------------|---------------|------------------|----------|-------------|-----------------|-----------------|-----------------|----------------|
| uuid                 | uuid        | uuid          | varchar(1024)    | char(16) | timestamp   | timestamp, NULL | timestamp, NULL | timestamp, NULL | time, NULL     |
| PK default random v4 | FK accounts | FK ambulances |                  |          |             |                 |                 |                 |                |

- unique index on (user_id, ambulance_id)
- index on arrived_at
- index on (ambulance_id, last_calculated)

### ETA notifications

| tracking_id               | notify_at_eta | fulfilled | phone id         |
|---------------------------|---------------|-----------|------------------|
| uuid                      | time          | bool      | uuid             |
| FK live tracking sessions |               |           | FK phone numbers |

- index on tracking_id
- index on (tracking_id, fulfilled, notify_at_eta)


# Data archive

### Ambulance Locations

| ambulance_id | ambulance_name     | location       | time      |
|--------------|--------------------|----------------|-----------|
| uuid         | varchar(255), null | WGS84 long/lat | timestamp |

### ETAs

| ambulance_id | current_location | destination    | eta       | calculated_at |
|--------------|------------------|----------------|-----------|---------------|
| uuid         | WGS84 long/lat   | WGS84 long/lat | timestamp | timestamp     |


---

# NoSQL schema (obsolete)

### Users
```
userid => {
    username
    password hash
    password salt
    role
    password reset needed
    settings {
        hospital location { lat, lon }
        default ETA time
    }
    phone number id => {
        phone number
        label
    }
    /* need to loop in order to remove, deleting ETAs somewhat complicated */
    tracking sessions => {
        last accessed
        ambulances => {
            description
            urgency
        }
    }       
}
```

### Ambulances
```
ambulance id => {
    location [ { lat, lon, last update } ]
    etas [
        to location { lat, lon }
        eta
        last updated
        notify numbers [ { user id, phone number id, fulfilled } ]
        notify at time remaining
    ]
}
```

### Audit logs
```
logs {
    timestamp
    ip address
    ambulance accessed
    username
}
```