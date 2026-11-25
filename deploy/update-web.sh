#!/usr/bin/env bash
# Update an existing, authenticated installation without touching its secrets,
# Nginx, certificates, firewall or systemd service definition.
set -Eeuo pipefail
umask 077
[[ "$(id -u)" == 0 ]] || { echo "Run with sudo bash." >&2; exit 1; }

action="${1:-}"
release="${2:-}"
root_dir="/opt/hghub-web"

require_gate() {
    local port="$1" headers api
    headers="$(curl --silent --show-error --max-time 5 --dump-header - \
        --output /dev/null "http://127.0.0.1:${port}/")"
    [[ "$headers" =~ ^HTTP/[0-9.]+[[:space:]]303[[:space:]] \
        && "$headers" == *"location: /login"* ]] \
        || { echo "Anonymous home is not gated." >&2; return 1; }
    api="$(curl --silent --show-error --max-time 5 --output /dev/null \
        --write-out '%{http_code}' --request POST \
        --header 'Content-Type: application/json' --data '{}' \
        "http://127.0.0.1:${port}/api/invoke/api_feed")"
    [[ "$api" == 401 ]] || { echo "Anonymous API is not denied." >&2; return 1; }
}

config_snapshot() {
    sha256sum /etc/hghub-web.env /etc/nginx/sites-available/hghub-web \
        /etc/systemd/system/hghub-web.service
}

restore_backup() {
    local backup="$1"
    [[ "$backup" =~ ^/opt/hghub-web/rollback/[0-9]{8}T[0-9]{6}Z-[a-zA-Z0-9.-]+$ ]] \
        && [[ -f "$backup/hghub-web" && -d "$backup/dist" ]] \
        || { echo "Invalid rollback directory." >&2; return 1; }
    install -m 0755 "$backup/hghub-web" "$root_dir/hghub-web.restore"
    mv "$root_dir/hghub-web.restore" "$root_dir/hghub-web"
    if [[ -d "$root_dir/dist" ]]; then
        [[ ! -e "$backup/failed-dist" ]] || return 1
        mv "$root_dir/dist" "$backup/failed-dist"
    fi
    mv "$backup/dist" "$root_dir/dist"
    systemctl restart hghub-web
    require_gate 8900
    echo "ROLLBACK_OK"
}

if [[ "$action" == rollback ]]; then
    restore_backup "$release"
    exit
fi

[[ "$release" =~ ^[0-9a-f]{40}$ ]] || { echo "Expected a full Git commit SHA." >&2; exit 1; }
candidate="$root_dir/candidates/$release"
unit="hghub-web-check-${release:0:12}"
data_dir="/var/lib/$unit"

case "$action" in
    prepare)
        expected_sha="${3:-}"
        [[ "$expected_sha" =~ ^[0-9a-f]{64}$ ]] || exit 1
        archive="/home/ubuntu/hghub-updates/$release/HGhub-web-linux-x86_64.tar.gz"
        [[ -f "$archive" && ! -e "$candidate" ]] || exit 1
        require_gate 8900
        actual_sha="$(sha256sum "$archive")"
        [[ "${actual_sha%% *}" == "$expected_sha" ]] || { echo "Bundle hash mismatch." >&2; exit 1; }
        while IFS= read -r entry; do
            entry="${entry#./}"
            [[ "$entry" != *"/../"* && "$entry" != ../* && "$entry" != /* ]] || exit 1
            case "$entry" in
                ""|hghub-web|dist|dist/|dist/*|deploy|deploy/|deploy/*) ;;
                *) echo "Unexpected bundle entry." >&2; exit 1 ;;
            esac
        done < <(tar -tzf "$archive")
        [[ -z "$(tar -tvzf "$archive" | awk 'substr($1,1,1) ~ /[lh]/ {print $1}')" ]] \
            || { echo "Bundle links are not accepted." >&2; exit 1; }
        install -d -m 0755 "$root_dir/candidates" "$candidate"
        tar --no-same-owner --no-same-permissions -xzf "$archive" -C "$candidate"
        [[ -f "$candidate/hghub-web" && -f "$candidate/dist/index.html" ]] || exit 1
        chmod 0755 "$candidate/hghub-web"
        find "$candidate/dist" -type d -exec chmod 0755 {} +
        find "$candidate/dist" -type f -exec chmod 0644 {} +
        if ldd "$candidate/hghub-web" | grep -q 'not found'; then
            echo "Missing runtime libraries; production is unchanged." >&2
            exit 1
        fi
        install -d -o hghub -g hghub -m 0700 "$data_dir"
        systemd-run --quiet --collect --unit="$unit" \
            --property=User=hghub --property=Group=hghub \
            --property=EnvironmentFile=/etc/hghub-web.env \
            --property=NoNewPrivileges=true --property=PrivateTmp=true \
            --property=ProtectSystem=strict --property=ProtectHome=true \
            --property="ReadWritePaths=$data_dir" \
            /usr/bin/env HGHUB_WEB_BIND=127.0.0.1:8901 HGHUB_STRICT=true \
            "HGHUB_WEB_DIST=$candidate/dist" "HGHUB_WEB_DATA=$data_dir" \
            "$candidate/hghub-web"
        ready=false
        for _ in {1..20}; do
            if require_gate 8901 2>/dev/null; then ready=true; break; fi
            sleep 1
        done
        [[ "$ready" == true ]] || { echo "Candidate did not become ready." >&2; exit 1; }
        echo "CANDIDATE_READY http://127.0.0.1:8901"
        sha256sum "$candidate/hghub-web"
        ;;
    activate)
        [[ -f "$candidate/hghub-web" && -d "$candidate/dist" ]] || exit 1
        require_gate 8901
        before_config="$(config_snapshot)"
        install -d -m 0700 "$root_dir/rollback"
        backup="$(mktemp -d "$root_dir/rollback/$(date -u +%Y%m%dT%H%M%SZ)-${release:0:12}.XXXXXX")"
        cp -a "$root_dir/hghub-web" "$backup/hghub-web"
        cp -a "$root_dir/dist" "$backup/dist"
        next_dist="$root_dir/dist.next-$release"
        [[ ! -e "$next_dist" ]] || exit 1
        cp -a "$candidate/dist" "$next_dist"
        install -m 0755 "$candidate/hghub-web" "$root_dir/hghub-web.next"
        rollback_on_error() {
            local code="$?"
            trap - ERR
            echo "Update failed; restoring the previous installation." >&2
            restore_backup "$backup" || echo "ROLLBACK_FAILED; inspect service before continuing." >&2
            exit "$code"
        }
        trap rollback_on_error ERR
        # Retain both versions; never overwrite an executing binary in place.
        mv "$root_dir/hghub-web.next" "$root_dir/hghub-web"
        mv "$root_dir/dist" "$backup/original-dist"
        mv "$next_dist" "$root_dir/dist"
        systemctl restart hghub-web
        ready=false
        for _ in {1..20}; do
            if require_gate 8900 2>/dev/null; then ready=true; break; fi
            sleep 1
        done
        [[ "$ready" == true ]]
        [[ "$(config_snapshot)" == "$before_config" ]]
        systemctl is-active --quiet hghub-web nginx
        trap - ERR
        echo "ACTIVATED $release"
        echo "ROLLBACK_DIR=$backup"
        sha256sum "$root_dir/hghub-web"
        ;;
    finish)
        systemctl stop "$unit"
        echo "Candidate stopped; production and rollback files are retained."
        ;;
    *)
        echo "Usage: update-web.sh prepare COMMIT ARCHIVE_SHA256 | activate COMMIT | finish COMMIT | rollback BACKUP_DIR" >&2
        exit 2
        ;;
esac
