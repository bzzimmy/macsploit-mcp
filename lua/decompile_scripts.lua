-- Fallback for dump_scripts: MacSploit's remote decompiler (~1s per script), within a time budget.
-- Expects `ids` (1-based indices into the last collection) to be defined before this chunk.
local scripts = getgenv().__macsploit_mcp_scripts
local deadline = os.clock() + 45
local sources = {}
for _, id in ids do
	if os.clock() > deadline then break end
	local ok, source = pcall(decompile, scripts[id])
	if ok and not source:find("^%-%- api request error") and not source:find("^%-%- Failed") then
		sources[tostring(id)] = source
	end
end
return sources
