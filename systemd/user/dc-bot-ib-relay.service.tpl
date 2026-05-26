[Unit]
Description=dc_bot IB options relay (headless)
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
WorkingDirectory=__REPO_ROOT__
ExecStart=__RELAY_BIN__ --headless
Restart=always
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target