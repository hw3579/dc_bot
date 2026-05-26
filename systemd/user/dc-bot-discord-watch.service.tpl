[Unit]
Description=dc_bot Discord order watcher
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
WorkingDirectory=__REPO_ROOT__
ExecStart=__UV_BIN__ run discord-watch-orders
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target