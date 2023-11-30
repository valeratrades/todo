- [ ] copy primeagen's code over for displaying history with htmx

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
```
