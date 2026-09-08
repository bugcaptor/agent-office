on run argv
	set creationAttempted to false
	try
		if application id "com.googlecode.iterm2" is not running then return "fallback"
		tell application id "com.googlecode.iterm2"
			if (count of windows) is 0 then
				set creationAttempted to true
				create window with default profile command (item 1 of argv)
			else
				set targetWindow to current window
				set creationAttempted to true
				tell targetWindow to create tab with default profile command (item 1 of argv)
			end if
			try
				activate
			end try
		end tell
		return "opened"
	on error message number code
		if creationAttempted then return "uncertain:" & code & ":" & message
		return "fallback:" & code & ":" & message
	end try
end run
