-- macsploit-mcp bridge --
local genv = getgenv()
if genv.__macsploit_mcp then return end
genv.__macsploit_mcp = true

if not game:IsLoaded() then game.Loaded:Wait() end
local Players = game:GetService("Players")
while not Players.LocalPlayer do Players:GetPropertyChangedSignal("LocalPlayer"):Wait() end

local HttpService = game:GetService("HttpService")
local BASE = "http://127.0.0.1:8766"
local HEADERS = {["Content-Type"] = "application/json"}
local baseEnv = getfenv(1)

local function post(path, body)
	return request({Url = BASE .. path, Method = "POST", Headers = HEADERS, Body = HttpService:JSONEncode(body)})
end

local function show(value)
	if type(value) == "table" then
		local ok, json = pcall(HttpService.JSONEncode, HttpService, value)
		if ok then return json end
	end
	return tostring(value)
end

local function run(job)
	local output = {}
	local function capture(kind)
		return function(...)
			local args = table.pack(...)
			for i = 1, args.n do args[i] = tostring(args[i]) end
			table.insert(output, kind .. ": " .. table.concat(args, " ", 1, args.n))
		end
	end
	local env = setmetatable({print = capture("print"), warn = capture("warn")}, {__index = baseEnv, __newindex = baseEnv})
	local result = {id = job.id, ok = false, output = output, returns = {}}
	local fn, err = loadstring(job.code)
	if fn then
		setfenv(fn, env)
		local packed = table.pack(pcall(fn))
		result.ok = packed[1]
		if packed[1] then
			for i = 2, packed.n do table.insert(result.returns, show(packed[i])) end
		else
			result.error = tostring(packed[2])
		end
	else
		result.error = err
	end
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
			backoff = math.min(backoff * 2, 10)
		end
	end
end)
