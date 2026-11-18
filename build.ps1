# Builds the release exe into .\dist
New-Item -ItemType Directory -Force -Path dist | Out-Null

cargo build --release
if ($LASTEXITCODE -ne 0) { exit 1 }
Copy-Item target\release\wuwa-apr.exe dist\wuwa-apr.exe -Force
Get-ChildItem dist
