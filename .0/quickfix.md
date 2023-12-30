- [ ] ws usage stats (in todos)

- [ ] reconfigure nginx. Just put the following into `sites-available`

server {

  listen 80 default_server;
  listen [::]:80 default_server;

} server {

  listen 80 default_server;

} listen [::]:80 default_server;


location / {
  proxy_pass http://localhost:8080;

server_name valeratrades.com www.valeratrades.com;
``` 

and then add also
```
root /home/valera/s/site

can have a state for the rm system, that it would watch and manage ALL the new trades according to the management sytle set

- [ ] implement correctly flags for todo  so it's not `taddo` but `tadd -o`, and not `tevo -y` but ` tev -oy`

// I say fuck bash, let's move what is possible into rust; using clap.

# Evening
- [ ] add copilot to neovim
