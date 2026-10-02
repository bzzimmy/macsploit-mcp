-- Macsploit-mcp bridge.
local genv = getgenv()
if genv.__macsploit_mcp then return end
genv.__macsploit_mcp = true

if not game:IsLoaded() then game.Loaded:Wait() end
local Players = game:GetService("Players")
while not Players.LocalPlayer do Players:GetPropertyChangedSignal("LocalPlayer"):Wait() end

local HttpService = game:GetService("HttpService")
local GuiService = game:GetService("GuiService")
local BASE = "http://127.0.0.1:8766"
local HEADERS = {["Content-Type"] = "application/json"}
local baseEnv = getfenv(1)

local function post(path, body)
	return request({Url = BASE .. path, Method = "POST", Headers = HEADERS, Body = HttpService:JSONEncode(body)})
end

-- Converts a value into something JSONEncode accepts: Instances become paths, other userdata strings.
local function plain(value, seen)
	local kind = typeof(value)
	if kind == "Instance" then return value:GetFullName() end
	if kind == "number" then return (value == value and math.abs(value) ~= math.huge) and value or tostring(value) end
	if kind == "string" or kind == "boolean" or kind == "nil" then return value end
	if kind ~= "table" then return tostring(value) end
	if seen[value] then return "<cycle>" end
	seen[value] = true
	local count = 0
	for _ in pairs(value) do count += 1 end
	local isArray = count == #value
	local out = {}
	for k, v in pairs(value) do
		out[isArray and k or tostring(k)] = plain(v, seen)
	end
	seen[value] = nil
	return out
end

-- JSON text for one return value; wrapping in an array lets nil encode as null.
local function encode(value)
	local ok, json = pcall(HttpService.JSONEncode, HttpService, {plain(value, {})})
	if not ok then json = HttpService:JSONEncode({tostring(value)}) end
	json = json:sub(2, -2)
	return json == "" and "null" or json
end

-- Scripts keep running after a kick, so report it: the game is dead until a rejoin.
local function disconnected()
	local ok, reason = pcall(function()
		if GuiService:GetErrorType().Name ~= "DisconnectErrors" then return nil end
		return GuiService:GetErrorCode().Name .. ": " .. GuiService:GetErrorMessage()
	end)
	return ok and reason or nil
end

local function run(job)
	local output = {}
	-- Capture print/warn for the result while still writing to the Roblox console.
	local function capture(kind, original)
		return function(...)
			original(...)
			local args = table.pack(...)
			for i = 1, args.n do args[i] = tostring(args[i]) end
			table.insert(output, kind .. ": " .. table.concat(args, " ", 1, args.n))
		end
	end
	local env = setmetatable({print = capture("print", print), warn = capture("warn", warn)}, {__index = baseEnv, __newindex = baseEnv})
	local result = {id = job.id, ok = false, output = output, returns = {}}
	local fn, err = loadstring(job.code)
	if fn then
		setfenv(fn, env)
		local packed = table.pack(pcall(fn))
		result.ok = packed[1]
		if packed[1] then
			for i = 2, packed.n do table.insert(result.returns, encode(packed[i])) end
		else
			local err = packed[2]
			result.error = type(err) == "string" and err or encode(err)
		end
	else
		result.error = err
	end
	result.disconnected = disconnected()
	pcall(post, "/result", result)
end

task.spawn(function()
	local backoff = 1
	while genv.__macsploit_mcp do
		local ok, res = pcall(post, "/poll", {})
		if ok and res.StatusCode == 200 then
			backoff = 1
			task.spawn(run, HttpService:JSONDecode(res.Body))
		elseif ok and res.StatusCode == 204 then
			backoff = 1
		else
			task.wait(backoff)
			backoff = math.min(backoff * 2, 5)
		end
	end
end)
