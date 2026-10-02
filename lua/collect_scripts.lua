-- Collects client script bytecode for dump_scripts.
local internal = {CorePackages = true, CoreGui = true, ["Script Context"] = true}
local seen, list = {}, {}

local function segments(instance)
	local names = {}
	while instance and instance ~= game do
		table.insert(names, 1, instance.Name)
		instance = instance.Parent
	end
	if not instance then table.insert(names, 1, "_nil") end
	return names
end

local function add(instance)
	if seen[instance] or not instance:IsA("LuaSourceContainer") then return end
	seen[instance] = true
	local isServer = instance:IsA("Script") and not instance:IsA("LocalScript")
		and instance.RunContext ~= Enum.RunContext.Client
	if isServer then return end
	local path = segments(instance)
	if internal[path[1]] then return end
	local ok, bytecode = pcall(getscriptbytecode, instance)
	if not ok or not bytecode or #bytecode == 0 then return end
	table.insert(list, {path = path, class = instance.ClassName, bytecode = base64encode(bytecode)})
end

for _, instance in game:GetDescendants() do add(instance) end
for _, instance in getnilinstances() do
	add(instance)
	for _, child in instance:GetDescendants() do add(child) end
end

return {placeId = game.PlaceId, player = game:GetService("Players").LocalPlayer.Name, scripts = list}
