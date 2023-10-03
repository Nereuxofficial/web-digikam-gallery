# web-digikam-gallery

# Running the project
## Set up environment
Put a `.env` file in the directory you execute the binary in(normally the project directory) and put the following content in it:
```
DATABASE_URL="sqlite:/PATH/TO/digikam4.db"
ADDR="127.0.0.1:8080"
SUBFOLDER="OPTIONAL_SUBFOLDER/"
BOTTOM_TEXT='<p>Your custom html at the bottom of the page here</p>'
```

After [installing Rust](https://rustup.rs), run the following command in the project directory:
```
cargo run --release
```