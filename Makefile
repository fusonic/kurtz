# Development helpers. Run `make help` for an overview.

COMPOSE := docker compose -f compose.dev.yml
TEST_ENV := KURTZ_TEST_POSTGRES_URL=postgres://kurtz:kurtz@127.0.0.1:5432/kurtz_test \
            KURTZ_TEST_MYSQL_URL=mysql://kurtz:kurtz@127.0.0.1:3306/kurtz_test

# `make start DB=mysql` runs the app against MySQL instead of PostgreSQL.
DB ?= postgres
ifeq ($(DB),mysql)
  export KURTZ_DEV_DATABASE_URL := mysql://kurtz:kurtz@mysql:3306/kurtz
endif
# Allow `make start KURTZ_DEV_LOG_LEVEL=…`.
export KURTZ_DEV_LOG_LEVEL

.PHONY: help start start-prod stop reset db run seed test lint fmt

help: ## Show this help
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  %-10s %s\n", $$1, $$2}'

start: ## Build and run the app plus databases in the foreground (Ctrl+C stops the app)
	@echo "Kurtz: http://localhost:8080/admin (password: dev). The first build takes a few minutes."
	$(COMPOSE) up --no-attach postgres --no-attach mysql

start-prod: db ## Build the release image and run it against the dev databases (foreground)
	docker build -t kurtz:local .
	@echo "Kurtz (release build): http://localhost:8080/admin (password: dev)"
	docker run --rm --name kurtz-prod --network kurtz-dev_default -p 8080:8080 \
	  -e DATABASE_URL=$${KURTZ_DEV_DATABASE_URL:-postgres://kurtz:kurtz@postgres:5432/kurtz} \
	  -e ADMIN_PASSWORD='$$argon2i$$v=19$$m=4096,t=3,p=1$$ZTY1OWY0Y2U3NjZiOGZkMjJlMTlkOWIyZDI5ZTNmMzU$$NZhOC45NwY930Q9yK0vY0iV2Den3IOpVCDUNCv3mF6Q' \
	  -e SESSION_SECRET=dev-only-secret-dev-only-secret-dev-only-secret-dev-only-secret! \
	  -e COOKIE_SECURE=false \
	  kurtz:local

stop: ## Stop the dev stack (data and build cache are kept)
	$(COMPOSE) down

reset: ## Stop the dev stack and delete database data and build cache
	$(COMPOSE) down -v

db: ## Start only the databases (for `make run` or `make test` without the app container)
	$(COMPOSE) up -d --wait postgres mysql

run: ## Run Kurtz natively with development settings (needs `make db`, port 8080 must be free)
	DATABASE_URL=postgres://kurtz:kurtz@127.0.0.1:5432/kurtz \
	ADMIN_PASSWORD='$$argon2i$$v=19$$m=4096,t=3,p=1$$ZTY1OWY0Y2U3NjZiOGZkMjJlMTlkOWIyZDI5ZTNmMzU$$NZhOC45NwY930Q9yK0vY0iV2Den3IOpVCDUNCv3mF6Q' \
	SESSION_SECRET=dev-only-secret-dev-only-secret-dev-only-secret-dev-only-secret! \
	COOKIE_SECURE=false \
	LAST_USED_FLUSH_SECS=5 \
	LOG_LEVEL=debug \
	cargo run

seed: ## Import dev/demo.csv (20 links) into the running dev app on localhost:8080 (password "dev")
	@jar=$$(mktemp) && \
	curl -fsS -c $$jar -o /dev/null -d password=dev http://localhost:8080/admin/login && \
	csrf=$$(curl -fsS -b $$jar http://localhost:8080/admin | grep -o 'name="_csrf" value="[^"]*"' | head -1 | sed 's/.*value="//;s/"//') && \
	curl -fsS -b $$jar -F "_csrf=$$csrf" -F action=import -F conflict=skip -F file=@dev/demo.csv http://localhost:8080/admin/import | grep -oE "Import finished|Nothing was imported" ; \
	rm -f $$jar

test: ## Run all tests against the dev databases (uses the kurtz_test databases)
	$(TEST_ENV) cargo test

lint: ## Check formatting and run clippy like CI does
	cargo fmt --all --check
	cargo clippy --all-targets --locked -- -D warnings

fmt: ## Format the code
	cargo fmt --all
