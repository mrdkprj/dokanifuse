# dokanifuse

ifuse for Windows(Dokan Fuse + libimobiledevice)  

Afc
```
ifuse.exe c:\mount_point
```

House arrest
```
ifuse.exe c:\mount_point -d com.example.ios
```

Print apps
```
ifuse.exe -a
```

Print apps with UIFileSharingEnabled
```
ifuse.exe -s
```

> [!NOTE]
> You need to open a command window in Admin mode to run the command.  
> To unmount, just press Ctrl+C or open another command window in Admin mode and run
> ```
> dokanctl.exe /u mount_point
> ```