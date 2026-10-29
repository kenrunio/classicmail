@echo off
REM Runs the latest ClassicMail code on your real data (close the installed ClassicMail first).
cd /d "%~dp0"
node scripts\preview.mjs
if errorlevel 1 pause
