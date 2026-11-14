CREATE TABLE links (
    id           BIGINT        NOT NULL AUTO_INCREMENT PRIMARY KEY,
    slug         VARCHAR(64)   NOT NULL,
    target_url   VARCHAR(2048) NOT NULL,
    target_hash  CHAR(64)      NOT NULL,
    enabled      BOOLEAN       NOT NULL DEFAULT TRUE,
    created_at   DATETIME(6)   NOT NULL,
    updated_at   DATETIME(6)   NOT NULL,
    last_used_at DATETIME(6)   NULL,
    UNIQUE INDEX links_slug_key (slug),
    INDEX links_target_hash_idx (target_hash),
    INDEX links_created_at_idx (created_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_unicode_ci;

-- Tags: free-form labels, several per link, stored normalised (lowercase).
CREATE TABLE link_tags (
    link_id BIGINT      NOT NULL,
    tag     VARCHAR(32) NOT NULL,
    PRIMARY KEY (link_id, tag),
    INDEX link_tags_tag_idx (tag),
    CONSTRAINT link_tags_link_fk FOREIGN KEY (link_id) REFERENCES links (id) ON DELETE CASCADE
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_unicode_ci;
