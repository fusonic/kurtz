CREATE TABLE links (
    id           BIGSERIAL PRIMARY KEY,
    slug         VARCHAR(64)   NOT NULL UNIQUE,
    target_url   VARCHAR(2048) NOT NULL,
    target_hash  CHAR(64)      NOT NULL,
    enabled      BOOLEAN       NOT NULL DEFAULT TRUE,
    created_at   TIMESTAMPTZ   NOT NULL,
    updated_at   TIMESTAMPTZ   NOT NULL,
    last_used_at TIMESTAMPTZ   NULL
);

CREATE INDEX links_target_hash_idx ON links (target_hash);
CREATE INDEX links_created_at_idx ON links (created_at);

-- Tags: free-form labels, several per link, stored normalised (lowercase).
CREATE TABLE link_tags (
    link_id BIGINT      NOT NULL REFERENCES links (id) ON DELETE CASCADE,
    tag     VARCHAR(32) NOT NULL,
    PRIMARY KEY (link_id, tag)
);

CREATE INDEX link_tags_tag_idx ON link_tags (tag);
