# Static-file serve of the pre-built wasm bundle. The wasm is built outside the
# Docker context (so we don't reinstall the entire Rust toolchain inside the
# image just to deploy) and copied straight into nginx. Fly's build context is
# `.dockerignore`-aware — see that file for what's excluded.
FROM nginx:alpine

# wasm needs the correct MIME type or browsers refuse to instantiate it; the
# default nginx config doesn't know `application/wasm`. Same for `.js` modules
# (default works but explicit is clearer) and Brotli precompressed assets if
# trunk emitted them. Also set a permissive CORS header in case the leaderboard
# JS ever wants to fetch the page resources from a different origin.
RUN { \
        echo 'types {'; \
        echo '  application/wasm wasm;'; \
        echo '  application/javascript js mjs;'; \
        echo '  text/html html;'; \
        echo '  font/ttf ttf;'; \
        echo '  image/png png;'; \
        echo '  image/svg+xml svg;'; \
        echo '}'; \
    } > /etc/nginx/conf.d/wasm.types

COPY <<'EOF' /etc/nginx/conf.d/default.conf
server {
    listen 80;
    listen [::]:80;
    server_name _;
    root /usr/share/nginx/html;
    index index.html;

    # Bevy/wgpu need cross-origin isolation for SharedArrayBuffer if we ever
    # enable threading; harmless to set now and avoids a redeploy later.
    add_header Cross-Origin-Opener-Policy same-origin;
    add_header Cross-Origin-Embedder-Policy require-corp;
    add_header Cache-Control "public, max-age=3600";

    # Long-cache the content-hashed wasm/js; the html shell short-caches above.
    location ~* "\.(wasm|js)$" {
        add_header Cache-Control "public, max-age=31536000, immutable";
    }

    gzip on;
    gzip_types application/wasm application/javascript text/html image/svg+xml;
    gzip_min_length 256;

    location / {
        try_files $uri /index.html;
    }
}
EOF

COPY dist/ /usr/share/nginx/html/
EXPOSE 80
