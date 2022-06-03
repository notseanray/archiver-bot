package main

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"log"
	"os"
	"os/exec"
	"os/signal"
	"strings"
	"syscall"

	"github.com/bwmarrin/discordgo"
	"gopkg.in/yaml.v3"
)

var config = Config{}
var lock = false
var chunksize = 4000
var downloader = "aria2c"

type Config struct {
	TOKEN string
}

func messageCreate(s *discordgo.Session, m *discordgo.MessageCreate) {

	// Ignore all messages created by the bot itself
	// This isn't required in this specific example but it's a good practice.
	if m.Author.ID == s.State.User.ID {
		return
	}
	// If the message is "ping" reply with "Pong!"
	if m.Content == "channels" {
		if lock {
			s.ChannelMessageSend(m.ChannelID, "locked, please wait for this to unlock")
			return
		}
		lock = true
		guild, err := s.Guild(m.GuildID)
		if err != nil {
			fmt.Println("failed to fetch guild", m.GuildID)
		}
		s.State.GuildAdd(guild)
		getChannels(s, m.GuildID)
	}
	if m.Content == "ping" {
		fmt.Println("pong")
	}
}

// filter by type https://pkg.go.dev/github.com/bwmarrin/discordgo#MessageType

// do we need to save extra properties of the author such as nickname?
// if a thread was started from the message, do we need to save it?

type SavableReferencedMessage struct {
	Id          string
	Author      SavableAuthor
	Attachments []SavableAttachment
	Content     string
	Pinned      bool
}

type SavableAuthor struct {
	Username      string
	Discriminator string // 4 numbers after username
	Id            string
	Mfa           bool
	Bot           bool
	Avatar        string // must fetch, hash is returned normally instead of url
}

type SavableAttachment struct {
	Id         string
	Url        string
	Filename   string
	Size       int
	Ephermeral bool // means short lasting, we will fetch these first when downloading all attachments
}

type SavableMessage struct {
	Id                string
	Author            SavableAuthor
	Attachments       []SavableAttachment
	Pinned            bool
	Content           string
	ReferencedMessage []SavableReferencedMessage
}

func filterMessage(m *discordgo.Message) SavableMessage {
	author := SavableAuthor{
		Username:      m.Author.Username,
		Discriminator: m.Author.Discriminator,
		Id:            m.Author.ID,
		Mfa:           m.Author.MFAEnabled,
		Bot:           m.Author.Bot,
		Avatar:        m.Author.AvatarURL(""),
	}
	attachments := []SavableAttachment{}
	for _, a := range m.Attachments {
		att := SavableAttachment{
			Id:         a.ID,
			Url:        a.URL,
			Filename:   a.Filename,
			Size:       a.Size,
			Ephermeral: a.Ephemeral,
		}
		attachments = append(attachments, att)
	}
	refmsg := []SavableReferencedMessage{}
	if m.ReferencedMessage != nil {
		refattachments := []SavableAttachment{}
		for _, refa := range m.ReferencedMessage.Attachments {
			att := SavableAttachment{
				Id:         refa.ID,
				Url:        refa.URL,
				Filename:   refa.Filename,
				Size:       refa.Size,
				Ephermeral: refa.Ephemeral,
			}
			refattachments = append(refattachments, att)
		}
		refauth := SavableAuthor{
			Username:      m.ReferencedMessage.Author.Username,
			Discriminator: m.ReferencedMessage.Author.Discriminator,
			Id:            m.ReferencedMessage.Author.ID,
			Mfa:           m.ReferencedMessage.Author.MFAEnabled,
			Bot:           m.ReferencedMessage.Author.Bot,
			Avatar:        m.ReferencedMessage.Author.AvatarURL(""),
		}
		reffinal := SavableReferencedMessage{
			Id:          m.ReferencedMessage.ID,
			Author:      refauth,
			Attachments: refattachments,
			Content:     m.ReferencedMessage.Content,
			Pinned:      m.ReferencedMessage.Pinned,
		}
		refmsg = append(refmsg, reffinal)
	}
	msg := SavableMessage{
		Id:                m.ID,
		Author:            author,
		Attachments:       attachments,
		Content:           m.Content,
		Pinned:            m.Pinned,
		ReferencedMessage: refmsg,
	}
	return msg
}

func getChannels(s *discordgo.Session, id string) {
	channels, _ := s.GuildChannels(id)
	channelCount := len(channels)
	fmt.Println("running on", id)
	os.Mkdir(id, 0755)
	s.State.MaxMessageCount = 100
	for i, c := range channels {
		if c.Type != discordgo.ChannelTypeGuildText {
			continue
		}
		os.Mkdir(id+"/"+c.ID, 0755)
		// force add to state to keep cache
		s.State.ChannelAdd(c)
		fmt.Println("c", c.Name)
		_, err := s.State.Channel(c.ID)
		if err != nil {
			fmt.Println("error fetching", c.Name)
		}
		lastId := c.LastMessageID
		messageCache := []SavableMessage{}
		for {
			messages, err := s.ChannelMessages(c.ID, 100, lastId, "", "")
			if err != nil {
				fmt.Println("failed to fetch messages")
			}
			for _, m := range messages {
				filtered := filterMessage(m)
				messageCache = append(messageCache, filtered)
			}
			if len(messages) <= 1 {
				fmt.Println("completed: ", c.Name, fmt.Sprintf("[%d/%d]", i+1, channelCount))
				break
			}
			lastId = messages[len(messages)-1].ID
		}
		count := 0
		chunk := 0
		chunkCache := []SavableMessage{}
		downloadPrio := []string{}
		downloadCache := []string{}
		for _, m := range messageCache {
			if len(m.Attachments) > 0 {
				for _, a := range m.Attachments {
					if a.Ephermeral {
						downloadPrio = append(downloadPrio, fmt.Sprintf("%s|%s/%s/%s", a.Url, id, c.ID, a.Id))
					} else {
						downloadCache = append(downloadCache, fmt.Sprintf("%s|%s/%s/%s", a.Url, id, c.ID, a.Id))
					}
				}
			}
		}
		if len(messageCache) < 4000 {
			// fmt.Println(data)
			chunkContent, e := json.Marshal(messageCache)
			if e != nil {
				fmt.Println("invalid json data")
			}
			chunkFile := fmt.Sprintf("%s/%s/%d", id, c.ID, chunk)
			_ = os.NewFile(0755, chunkFile)
			err = ioutil.WriteFile(chunkFile, chunkContent, 0644)
			if err != nil {
				fmt.Println("failed to save chunk", chunk)
			}
			fmt.Println("saved chunk", chunk)
		} else {
			for _, m := range messageCache {
				count++
				chunkCache = append(chunkCache, m)
				if count%4000 == 0 {
					chunkContent, e := json.Marshal(chunkCache)
					if e != nil {
						fmt.Println("invalid json data")
					}
					chunkFile := fmt.Sprintf("%s/%s/%d", id, c.ID, chunk)
					_ = os.NewFile(0755, chunkFile)
					err = ioutil.WriteFile(chunkFile, chunkContent, 0755)
					if err != nil {
						fmt.Println("failed to save chunk", chunk)
					}
					fmt.Println("saved chunk", chunk)
					chunkCache = make([]SavableMessage, 0)
					chunk++
				}
			}
		}
		for _, d := range downloadPrio {
			down := strings.Split(d, "|")
			cmd := exec.Command(
				"aria2c",
				down[0],
				"-d",
				down[1],
			)
			cmd.Output()
		}
		for _, d := range downloadCache {
			down := strings.Split(d, "|")
			cmd := exec.Command(
				"aria2c",
				down[0],
				"-d",
				down[1],
			)
			cmd.Output()
		}
	}
	fmt.Println("completed")
	lock = false
}

func main() {
	fmt.Println("starting bot")
	contents, rerr := ioutil.ReadFile("./config.yaml")
	if rerr != nil {
		log.Fatal(rerr)
		os.Exit(1)
	}
	uerr := yaml.Unmarshal(contents, &config)
	if uerr != nil {
		log.Fatal(uerr)
		os.Exit(1)
	}
	dg, err := discordgo.New("Bot " + config.TOKEN)
	if err != nil {
		fmt.Println("error creating Discord session,", err)
		return
	}

	// Register the messageCreate func as a callback for MessageCreate events.
	dg.AddHandler(messageCreate)

	// In this example, we only care about receiving message events.
	dg.Identify.Intents = discordgo.IntentsGuildMessages

	// Open a websocket connection to Discord and begin listening.
	err = dg.Open()
	if err != nil {
		fmt.Println("error opening connection,", err)
		return
	}

	fmt.Println("Bot is now running.  Press CTRL-C to exit.")
	sc := make(chan os.Signal, 1)
	signal.Notify(sc, syscall.SIGINT, syscall.SIGTERM, os.Interrupt, os.Kill)
	<-sc

	dg.Close()
	fmt.Println("Closed connection to discord")
}
