@echo off
echo Testing RoboCopy performance on the same data...
echo.
robocopy "C:\Program Files (x86)\Steam\steamapps\common\Counter-Strike Global Offensive" "H:\stuff\backup\steam\test-robocopy" /E /MT:32 /R:0 /W:0

echo.
echo Now with more threads...
robocopy "C:\Program Files (x86)\Steam\steamapps\common\Team Fortress 2" "H:\stuff\backup\steam\test-robocopy2" /E /MT:128 /R:0 /W:0